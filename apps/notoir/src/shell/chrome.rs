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
                    let items =
                        menus::visible_items(&menus::expand(&menu.items, core), &core.commands);
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

pub(crate) fn menu_items(
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
            MenuItem::Entry { id, args, text } => {
                let Some(spec) = core.commands.get(&(*id).into()) else {
                    continue;
                };
                if ui
                    .add_enabled(
                        spec.enablement.eval(context),
                        egui::Button::new(text.as_str()),
                    )
                    .clicked()
                {
                    actions.push(
                        CommandInvocation::new(*id, InvocationSource::Menu).with_args(args.clone()),
                    );
                    ui.close();
                }
            }
            MenuItem::Dynamic(_) => {}
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

/// `[ Edit │ Split │ Preview ]` for the active Markdown document (spec 24).
fn markdown_modes(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    let Some(view) = core
        .workbench
        .active_view()
        .and_then(|v| core.workbench.view(v))
    else {
        return;
    };
    use notoir_core::view::ViewKind;
    let modes: [(ViewKind, &str, &'static str); 3] = if core.is_markdown(view.document_id) {
        [
            (ViewKind::Text, "markdown:edit", "markdown.showEditor"),
            (
                ViewKind::MarkdownSplit,
                "markdown:split",
                "markdown.showSplit",
            ),
            (
                ViewKind::MarkdownPreview,
                "markdown:preview",
                "markdown.showPreview",
            ),
        ]
    } else if core.is_yaml(view.document_id) {
        [
            (ViewKind::Text, "json:text", "yaml.showText"),
            (ViewKind::JsonSplit, "json:split", "yaml.showSplit"),
            (ViewKind::JsonTree, "json:tree", "yaml.showTree"),
        ]
    } else if core.is_structured(view.document_id) {
        [
            (ViewKind::Text, "json:text", "json.showText"),
            (ViewKind::JsonSplit, "json:split", "json.showSplit"),
            (ViewKind::JsonTree, "json:tree", "json.showTree"),
        ]
    } else {
        return;
    };
    ui.add_space(16.0);
    ui.spacing_mut().item_spacing.x = 0.0;
    for (kind, label, id) in modes {
        let selected = view.kind == kind;
        let text = RichText::new(t!(label)).size(12.0).color(if selected {
            kit.color(Tok::TextPrimary)
        } else {
            kit.color(Tok::TextSecondary)
        });
        if ui
            .add(egui::Button::selectable(selected, text).min_size(egui::vec2(56.0, 24.0)))
            .clicked()
            && !selected
        {
            actions.push(CommandInvocation::new(id, InvocationSource::Toolbar));
        }
    }
    ui.spacing_mut().item_spacing.x = 2.0;
    if core.is_structured(view.document_id) {
        ui.add_space(8.0);
        let format = egui::Button::new(RichText::new(t!("json:format")).size(12.0))
            .min_size(egui::vec2(56.0, 24.0));
        if ui.add(format).clicked() {
            actions.push(CommandInvocation::new(
                if core.is_yaml(view.document_id) {
                    "yaml.format"
                } else {
                    "json.format"
                },
                InvocationSource::Toolbar,
            ));
        }
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
                markdown_modes(ui, kit, core, actions);
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
                // MCP server (spec 29): `MCP ●2`.
                super::mcp_ui::status_item(ui, kit, core, actions);
                // Plugin items on the left (spec 19).
                super::plugins_ui::status_items(ui, kit, core, false, actions);
                // Git branch (spec 28): `⎇ main* ↑1 ↓2`.
                if let Some(branch) = core.git_branch_label() {
                    ui.add(kit.icon(
                        ui.ctx(),
                        Icon::GitBranch,
                        crate::theme::ICON_SM,
                        kit.color(Tok::TextSecondary),
                    ));
                    let resp = ui.add(
                        egui::Button::new(
                            RichText::new(&branch)
                                .font(crate::fonts::mono(12.0))
                                .color(kit.color(Tok::TextSecondary)),
                        )
                        .frame(false),
                    );
                    let a11y = t!("statusbar:branch", name = branch.as_str());
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a11y)
                    });
                    if resp
                        .on_hover_text(t!("commands:git.switchBranch"))
                        .clicked()
                    {
                        actions.push(CommandInvocation::new(
                            "git.switchBranch",
                            InvocationSource::StatusBar,
                        ));
                    }
                    ui.add_space(8.0);
                }
                // A network Git operation: "Pushing… 45%" with Cancel.
                if let Some(progress) = core.git_progress_label() {
                    ui.add(egui::Spinner::new().size(11.0));
                    let resp = ui.label(
                        RichText::new(&progress)
                            .size(12.0)
                            .color(kit.color(Tok::TextSecondary)),
                    );
                    let a11y = t!("statusbar:gitProgress", progress = progress.as_str());
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &a11y)
                    });
                    if icon_button_sized(
                        ui,
                        kit,
                        Icon::X,
                        &t!("commands:git.cancel"),
                        false,
                        true,
                        20.0,
                    )
                    .clicked()
                    {
                        actions.push(CommandInvocation::new(
                            "git.cancel",
                            InvocationSource::StatusBar,
                        ));
                    }
                    ui.add_space(8.0);
                }
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
                    super::plugins_ui::status_items(ui, kit, core, true, actions);
                    for item in document_items(core).into_iter().rev() {
                        ui.add_space(6.0);
                        let font = if item.mono {
                            crate::fonts::mono(12.0)
                        } else {
                            egui::FontId::proportional(12.0)
                        };
                        let text = RichText::new(item.text)
                            .font(font)
                            .color(kit.color(Tok::TextSecondary));
                        match item.command {
                            Some(cmd) => {
                                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                                    actions.push(CommandInvocation::new(
                                        cmd,
                                        InvocationSource::StatusBar,
                                    ));
                                }
                            }
                            None => {
                                ui.label(text);
                            }
                        }
                    }
                });
            });
        });
}

/// One status bar readout for the active document.
pub struct StatusItem {
    pub text: String,
    /// Numbers render in the monospace face (spec 21).
    pub mono: bool,
    /// Command run on click (pickers).
    pub command: Option<&'static str>,
}

/// Left-to-right: cursor position, indentation, encoding, line ending,
/// language, zoom (only when not 100 %).
pub fn document_items(core: &AppCore) -> Vec<StatusItem> {
    let wb = &core.workbench;
    let Some(view) = wb.active_view().and_then(|v| wb.view(v)) else {
        return Vec::new();
    };
    let Some(doc) = wb.document(view.document_id) else {
        return Vec::new();
    };
    // Settings / Welcome tabs have no text readouts.
    if matches!(doc.origin, notoir_core::DocumentOrigin::Virtual { .. }) {
        return Vec::new();
    }
    let indent_style = core.indent_style(doc.id);
    let tab = indent_style.tab_size;
    let rope = doc.buffer.rope();
    let head = view.selection.primary().head.min(rope.len_chars());
    let line = rope.char_to_line(head);
    let rel = head - rope.line_to_char(line);
    let col =
        notoir_editor::wrap::col_of(&notoir_editor::display_map::line_text(rope, line), rel, tab)
            + 1;
    let selected: usize = view
        .selection
        .ranges()
        .iter()
        .map(|r| r.to() - r.from())
        .sum();
    let mut position = t!("statusbar:lnCol", line = line + 1, col = col);
    if selected > 0 {
        position.push(' ');
        position.push_str(&t!("statusbar:selected", count = selected));
    }
    let indent = if indent_style.insert_spaces {
        t!("statusbar:spaces", size = tab)
    } else {
        t!("statusbar:tabSize", size = tab)
    };
    let language = if doc.language == notoir_editor::LanguageId::PLAINTEXT {
        t!("statusbar:plainText")
    } else {
        notoir_editor::language::definition(doc.language)
            .name
            .to_owned()
    };
    let mut items = vec![
        StatusItem {
            text: position,
            mono: true,
            command: Some("navigate.goToLine"),
        },
        StatusItem {
            text: indent,
            mono: false,
            command: Some("editor.changeIndentation"),
        },
        StatusItem {
            text: doc.encoding.label(),
            mono: false,
            command: Some("editor.changeEncoding"),
        },
        StatusItem {
            text: doc.line_ending.label().to_owned(),
            mono: false,
            command: Some("editor.changeLineEnding"),
        },
        StatusItem {
            text: language,
            mono: false,
            command: Some("editor.changeLanguageMode"),
        },
    ];
    let zoom = (core.zoom_for(view.id) * 100.0).round() as u32;
    if zoom != 100 {
        items.push(StatusItem {
            text: format!("{zoom}%"),
            mono: true,
            command: Some("view.zoomReset"),
        });
    }
    items
}
