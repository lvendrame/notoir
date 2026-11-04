//! Application-owned macOS menus. Keep native objects alive while AppKit tracks them.
use crate::{
    application::WindowInfo,
    core::AppCore,
    menus::{self, MenuItem},
    t,
};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, PredefinedMenuItem, Submenu};
use notoir_core::command::{CommandInvocation, CommandRegistry, CommandScope, InvocationSource};
use notoir_core::keymap::Platform;
use notoir_core::when::{ContextSnapshot, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
struct Action {
    invocation: CommandInvocation,
    text: String,
    enabled: bool,
    checked: bool,
    accelerator: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
enum Item {
    Action(Action),
    Submenu {
        key: &'static str,
        text: String,
        items: Vec<Item>,
    },
    Separator,
}

/// Context maps are unordered. Only resolved menu properties may determine changes.
/// Child submenu contents are reconciled within that submenu, not its parent.
fn same_structure(before: &[Item], after: &[Item]) -> bool {
    before.len() == after.len()
        && before.iter().zip(after).all(|(a, b)| match (a, b) {
            (Item::Action(a), Item::Action(b)) => a.invocation == b.invocation,
            (Item::Submenu { key: a, .. }, Item::Submenu { key: b, .. }) => a == b,
            (Item::Separator, Item::Separator) => true,
            _ => false,
        })
}

fn resolve_items(
    items: &[MenuItem],
    registry: &CommandRegistry<AppCore>,
    context: &ContextSnapshot,
    window: Option<&str>,
) -> Vec<Item> {
    items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Separator => Some(Item::Separator),
            MenuItem::Submenu { label, items } => Some(Item::Submenu {
                key: label,
                text: t!(label),
                items: resolve_items(items, registry, context, window),
            }),
            MenuItem::Command {
                id,
                args,
                label,
                checked,
            } => {
                let spec = registry.get(&(*id).into())?;
                Some(Item::Action(Action {
                    invocation: CommandInvocation::new(*id, InvocationSource::Menu)
                        .with_args(args.clone()),
                    text: label
                        .as_deref()
                        .map_or_else(|| t!(&spec.title_key), |key| t!(key)),
                    enabled: if window.is_some() {
                        spec.enablement.eval(context)
                    } else {
                        spec.scope == CommandScope::Application
                    },
                    checked: checked
                        .as_deref()
                        .is_some_and(|key| matches!(context.get(key), Some(Value::Bool(true)))),
                    // Editor shortcuts remain in egui; only the zero-window app
                    // needs native accelerators because no editor receives input.
                    accelerator: if window.is_none() {
                        match *id {
                            "file.newWindow" => Some("Super+Shift+N"),
                            "app.quit" => Some("Super+Q"),
                            _ => None,
                        }
                    } else {
                        None
                    },
                }))
            }
            MenuItem::Entry { id, args, text } => {
                let spec = registry.get(&(*id).into())?;
                Some(Item::Action(Action {
                    invocation: CommandInvocation::new(*id, InvocationSource::Menu)
                        .with_args(args.clone()),
                    text: text.clone(),
                    enabled: window.is_some() && spec.enablement.eval(context),
                    checked: *id == "window.activate" && window == args["id"].as_str(),
                    accelerator: None,
                }))
            }
            MenuItem::Dynamic(_) => None,
        })
        .collect()
}

struct NativeSubmenu {
    key: &'static str,
    text: String,
    handle: Submenu,
    model: Vec<Item>,
    items: Vec<NativeItem>,
}

enum NativeItem {
    Action(CheckMenuItem),
    Submenu(NativeSubmenu),
    Separator,
}

impl NativeItem {
    fn remove_actions(&self, actions: &mut HashMap<MenuId, CommandInvocation>) {
        match self {
            Self::Action(item) => {
                actions.remove(item.id());
            }
            Self::Submenu(menu) => {
                for item in &menu.items {
                    item.remove_actions(actions);
                }
            }
            Self::Separator => {}
        }
    }
}

impl NativeSubmenu {
    fn new(key: &'static str, text: String) -> Self {
        Self {
            key,
            handle: Submenu::new(&text, true),
            text,
            model: Vec::new(),
            items: Vec::new(),
        }
    }

    fn update(&mut self, model: Vec<Item>, actions: &mut HashMap<MenuId, CommandInvocation>) {
        if !same_structure(&self.model, &model) {
            // A real dynamic entry was added/removed (e.g. an open window or
            // recent file). Replace only this submenu's children, never the bar.
            while self.handle.remove_at(0).is_some() {}
            for item in &self.items {
                item.remove_actions(actions);
            }
            self.items = model
                .iter()
                .map(|item| match item {
                    Item::Separator => {
                        let _ = self.handle.append(&PredefinedMenuItem::separator());
                        NativeItem::Separator
                    }
                    Item::Submenu { key, text, items } => {
                        let mut menu = Self::new(key, text.clone());
                        menu.update(items.clone(), actions);
                        let _ = self.handle.append(&menu.handle);
                        NativeItem::Submenu(menu)
                    }
                    Item::Action(action) => {
                        let item = CheckMenuItem::new(
                            &action.text,
                            action.enabled,
                            action.checked,
                            action.accelerator.and_then(|s| s.parse().ok()),
                        );
                        actions.insert(item.id().clone(), action.invocation.clone());
                        let _ = self.handle.append(&item);
                        NativeItem::Action(item)
                    }
                })
                .collect();
        } else {
            for ((native, before), after) in self.items.iter_mut().zip(&self.model).zip(&model) {
                match (native, before, after) {
                    (NativeItem::Action(item), Item::Action(before), Item::Action(after)) => {
                        if before.text != after.text {
                            item.set_text(&after.text);
                        }
                        if before.enabled != after.enabled {
                            item.set_enabled(after.enabled);
                        }
                        if before.checked != after.checked {
                            item.set_checked(after.checked);
                        }
                        if before.accelerator != after.accelerator {
                            let _ = item
                                .set_accelerator(after.accelerator.and_then(|s| s.parse().ok()));
                        }
                    }
                    (NativeItem::Submenu(menu), _, Item::Submenu { text, items, .. }) => {
                        if menu.text != *text {
                            menu.handle.set_text(text);
                            menu.text.clone_from(text);
                        }
                        menu.update(items.clone(), actions);
                    }
                    _ => {}
                }
            }
        }
        self.model = model;
    }
}

pub struct NativeMenu {
    menu: Menu,
    submenus: Vec<NativeSubmenu>,
    actions: HashMap<MenuId, CommandInvocation>,
    target: Option<String>,
    registry: CommandRegistry<AppCore>,
    events: std::sync::mpsc::Receiver<MenuEvent>,
}

impl NativeMenu {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, events) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = tx.send(event);
            repaint.request_repaint_of(egui::ViewportId::ROOT);
        }));
        let menu = Menu::new();
        menu.init_for_nsapp();
        let mut registry = CommandRegistry::default();
        crate::commands::register_all(&mut registry, Platform::Mac);
        Self {
            menu,
            submenus: Vec::new(),
            actions: HashMap::new(),
            target: None,
            registry,
            events,
        }
    }

    pub fn update(
        &mut self,
        core: Option<&AppCore>,
        _windows: &[WindowInfo],
    ) -> Vec<(Option<String>, CommandInvocation)> {
        // Dispatch queued selections using the target for which the menu was shown.
        let mut actions = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if let Some(command) = self.actions.get(&event.id) {
                actions.push((self.target.clone(), command.clone()));
            }
        }
        self.target = core.and_then(|c| c.window_id.clone());
        let context = core.map(|c| c.context(false)).unwrap_or_default();
        let registry = core.map_or(&self.registry, |c| &c.commands);
        let models: Vec<_> = menus::menu_bar(Platform::Mac)
            .into_iter()
            .map(|menu| {
                let items = core
                    .map(|c| menus::expand(&menu.items, c))
                    .unwrap_or(menu.items);
                let items = menus::visible_items(&items, registry);
                (
                    menu.label,
                    t!(menu.label),
                    resolve_items(&items, registry, &context, self.target.as_deref()),
                )
            })
            .collect();
        // The top-level menu set is fixed by the platform menu definition.
        // Never remove/recreate it on editor focus, context, title or state changes.
        for (index, (key, text, items)) in models.into_iter().enumerate() {
            if index == self.submenus.len() {
                let menu = NativeSubmenu::new(key, text.clone());
                let _ = self.menu.append(&menu.handle);
                self.submenus.push(menu);
            }
            let menu = &mut self.submenus[index];
            debug_assert_eq!(menu.key, key);
            if menu.text != text {
                menu.handle.set_text(&text);
                menu.text = text;
            }
            menu.update(items, &mut self.actions);
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notoir_core::command::{Category, CommandOutcome, CommandSpec};
    use serde_json::json;

    fn registry() -> CommandRegistry<AppCore> {
        let mut registry = CommandRegistry::default();
        registry
            .register(
                CommandSpec::new("test.action", Category::Help, |_, _| CommandOutcome::Done)
                    .when("editorFocus && !readOnly")
                    .unwrap(),
            )
            .unwrap();
        registry
    }

    fn items() -> Vec<MenuItem> {
        vec![MenuItem::Entry {
            id: "test.action",
            args: json!({}),
            text: "Action".into(),
        }]
    }

    // spec 18: "Native menus survive editor context updates"
    #[test]
    fn rebuilt_context_maps_do_not_change_menu_structure() {
        let registry = registry();
        let mut before = ContextSnapshot::default();
        before
            .set_bool("editorFocus", true)
            .set_bool("readOnly", false)
            .set_num("cursorLine", 1.0);
        let original = resolve_items(&items(), &registry, &before, Some("first"));
        for line in 2..100 {
            let mut after = ContextSnapshot::default();
            after
                .set_num("cursorLine", f64::from(line))
                .set_bool("readOnly", false)
                .set_bool("editorFocus", true);
            let next = resolve_items(&items(), &registry, &after, Some("first"));
            assert_eq!(original, next);
            assert!(same_structure(&original, &next));
        }
        before.set_bool("readOnly", true);
        let disabled = resolve_items(&items(), &registry, &before, Some("first"));
        assert_ne!(original, disabled);
        assert!(same_structure(&original, &disabled));
        assert!(matches!(&disabled[0], Item::Action(action) if !action.enabled));
    }

    #[test]
    fn checked_state_and_localization_preserve_item_identity() {
        let before = Item::Action(Action {
            invocation: CommandInvocation::new("window.activate", InvocationSource::Menu)
                .with_args(json!({"id":"first"})),
            text: "First".into(),
            enabled: true,
            checked: false,
            accelerator: None,
        });
        let mut after = before.clone();
        if let Item::Action(action) = &mut after {
            action.checked = true;
            action.text = "Premier".into();
        }
        assert!(same_structure(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after)
        ));
        if let Item::Action(action) = &mut after {
            action.invocation.args = json!({"id":"second"});
        }
        assert!(!same_structure(&[before], &[after]));
    }

    #[test]
    fn dynamic_submenus_do_not_replace_their_parent() {
        let before = Item::Submenu {
            key: "menus:top.window",
            text: "Window".into(),
            items: Vec::new(),
        };
        let mut after = before.clone();
        if let Item::Submenu { items, .. } = &mut after {
            items.push(Item::Separator);
        }
        assert!(same_structure(&[before], &[after]));
    }
}
