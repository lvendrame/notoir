//! Menu bar, toolbar and status bar (docs/specs/18, 19).

use egui::{Align, Frame, Layout, Margin, RichText};
use notoir_core::command::{CommandId, CommandInvocation, InvocationSource};
use notoir_core::when::{ContextSnapshot, Value};

use super::{Actions, Kit, icon_button, icon_button_sized};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::menus::{self, MenuItem};
use crate::t;
use crate::theme::{ICON_SM, Tok};

pub const MENU_BAR_HEIGHT: f32 = 28.0;
pub const TOOLBAR_HEIGHT: f32 = 36.0;
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

fn bar_frame(kit: &Kit) -> Frame {
    Frame::new()
        .fill(kit.color(Tok::BgApp))
        .inner_margin(Margin::symmetric(8, 0))
}

fn shortcut_for(core: &AppCore, id: &str) -> String {
    core.keymap
        .chords_for(&CommandId::from(id.to_owned()))
        .first()
        .map(|c| c.display(core.platform))
        .unwrap_or_default()
}

fn is_true(ctx: &ContextSnapshot, key: &str) -> bool {
    matches!(ctx.get(key), Some(Value::Bool(true)))
}

pub fn menu_bar(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    context: &ContextSnapshot,
    actions: &mut Actions,
) {
    egui::Panel::top("menu-bar")
        .exact_size(MENU_BAR_HEIGHT)
        .frame(bar_frame(kit))
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for menu in menus::menu_bar(core.platform) {
                    let items = menus::visible_items(&menu.items, &core.commands);
                    ui.menu_button(t!(menu.label), |ui| {
                        ui.set_min_width(220.0);
                        if items.is_empty() {
                            ui.add_enabled(false, egui::Button::new(t!("menus:comingSoon")));
                        } else {
                            menu_items(ui, kit, core, context, &items, actions);
                        }
                    });
                }
            });
        });
}

fn menu_items(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    context: &ContextSnapshot,
    items: &[MenuItem],
    actions: &mut Actions,
) {
    for item in items {
        match item {
            MenuItem::Separator => {
                ui.separator();
            }
            MenuItem::Submenu { label, items } => {
                ui.menu_button(t!(label), |ui| {
                    ui.set_min_width(200.0);
                    menu_items(ui, kit, core, context, items, actions);
                });
            }
            MenuItem::Command {
                id,
                args,
                label,
                checked,
            } => {
                let Some(spec) = core.commands.get(&(*id).into()) else {
                    continue;
                };
                let text = label
                    .as_deref()
                    .map_or_else(|| t!(&spec.title_key), |k| t!(k));
                let is_checked = checked.as_deref().is_some_and(|k| is_true(context, k));
                let check_color = if is_checked {
                    kit.color(Tok::Accent)
                } else {
                    egui::Color32::TRANSPARENT
                };
                let check = kit.icon(ui.ctx(), Icon::Check, ICON_SM, check_color);
                let button = egui::Button::new((check, text)).shortcut_text(shortcut_for(core, id));
                let enabled = spec.enablement.eval(context);
                if ui.add_enabled(enabled, button).clicked() {
                    actions.push(
                        CommandInvocation::new(*id, InvocationSource::Menu).with_args(args.clone()),
                    );
                    ui.close();
                }
            }
        }
    }
}

/// Fixed minimal toolbar (spec 18); items appear as their commands exist.
const TOOLBAR: &[&[(&str, Icon)]] = &[
    &[
        ("view.togglePrimarySidebar", Icon::PanelLeft),
        ("file.newMenu", Icon::Plus),
        ("file.openMenu", Icon::FolderOpen),
        ("file.save", Icon::Save),
    ],
    &[("edit.undo", Icon::Undo2), ("edit.redo", Icon::Redo2)],
    &[("edit.find", Icon::Search)],
];
const TOOLBAR_END: &[(&str, Icon)] = &[("compare.open", Icon::GitCompare)];

fn toolbar_item(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    context: &ContextSnapshot,
    id: &str,
    icon: Icon,
    actions: &mut Actions,
) {
    let Some(spec) = core.commands.get(&CommandId::from(id.to_owned())) else {
        return;
    };
    let shortcut = shortcut_for(core, id);
    let tip = if shortcut.is_empty() {
        t!(&spec.title_key)
    } else {
        format!("{} ({shortcut})", t!(&spec.title_key))
    };
    let active = id == "view.togglePrimarySidebar" && core.panels.primary.visible;
    if icon_button(ui, kit, icon, &tip, active, spec.enablement.eval(context)).clicked() {
        actions.push(CommandInvocation::new(
            spec.id.clone(),
            InvocationSource::Toolbar,
        ));
    }
}

pub fn toolbar(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    context: &ContextSnapshot,
    actions: &mut Actions,
) {
    egui::Panel::top("toolbar")
        .exact_size(TOOLBAR_HEIGHT)
        .frame(bar_frame(kit))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut first = true;
                for group in TOOLBAR {
                    let present: Vec<_> = group
                        .iter()
                        .filter(|(id, _)| {
                            core.commands
                                .get(&CommandId::from(id.to_string()))
                                .is_some()
                        })
                        .collect();
                    if present.is_empty() {
                        continue;
                    }
                    if !first {
                        ui.add_space(10.0);
                    }
                    first = false;
                    for (id, icon) in present {
                        toolbar_item(ui, kit, core, context, id, *icon, actions);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (id, icon) in TOOLBAR_END {
                        toolbar_item(ui, kit, core, context, id, *icon, actions);
                    }
                });
            });
        });
}

pub fn status_bar(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    egui::Panel::bottom("status-bar")
        .exact_size(STATUS_BAR_HEIGHT)
        .frame(bar_frame(kit))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if let Some(first) = core.keymap.pending() {
                    let keys = first.display(core.platform);
                    ui.label(
                        RichText::new(t!("statusbar:chordPending", keys = keys))
                            .size(12.0)
                            .color(kit.color(Tok::TextSecondary)),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let unread = core.notifications.unread();
                    let tip = t!("statusbar:notifications");
                    let resp = icon_button_sized(ui, kit, Icon::Bell, &tip, false, true, 22.0);
                    if unread > 0 {
                        let pos = resp.rect.right_top() + egui::vec2(-5.0, 5.0);
                        ui.painter().circle_filled(pos, 3.5, kit.color(Tok::Accent));
                    }
                    if resp.clicked() {
                        actions.push(CommandInvocation::new(
                            "workbench.showNotifications",
                            InvocationSource::StatusBar,
                        ));
                    }
                });
            });
        });
}
