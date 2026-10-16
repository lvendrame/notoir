//! Plugin UI (docs/specs/30 §Permissions & Approval, §Errors): the approval
//! prompt shown before a plugin can run, the per-call prompt for
//! `notoir.process.run`, and the Plugin Output panel.

use egui::{Align, Key, Layout, RichText};
use notoir_core::command::{CommandInvocation, InvocationSource};
use serde_json::{Value, json};

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::t;
use crate::theme::Tok;

fn cmd(actions: &mut Actions, id: &'static str, args: Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

/// `✓ Read open documents` / `✕ Internet access`: an icon (the UI fonts lack
/// the marks) plus the text, never colour alone.
fn permission_row(ui: &mut egui::Ui, kit: &mut Kit, granted: bool, text: &str, size: f32) {
    let (icon, color) = if granted {
        (crate::icons::Icon::Check, kit.color(Tok::TextPrimary))
    } else {
        (crate::icons::Icon::X, kit.color(Tok::TextTertiary))
    };
    ui.horizontal(|ui| {
        ui.add(kit.icon(ui.ctx(), icon, crate::theme::ICON_SM, color));
        let resp = ui.label(RichText::new(text).size(size).color(color));
        let a11y = format!("{}  {text}", if granted { "✓" } else { "✕" });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &a11y));
    });
}

/// The approval prompt (`"Word Count" 1.0.0 requests: …`).
pub fn review(ctx: &egui::Context, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    let Some(r) = &core.plugins.review else {
        return;
    };
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        cmd(actions, "plugins.reviewAnswer", json!({ "approve": false }));
        return;
    }
    let m = &r.manifest;
    egui::Modal::new(egui::Id::new("plugin-review")).show(ctx, |ui| {
        ui.set_width(440.0);
        let title_key = if r.update {
            "plugins:review.titleUpdate"
        } else {
            "plugins:review.title"
        };
        ui.label(
            RichText::new(t!(
                title_key,
                name = m.plugin.name.as_str(),
                version = m.plugin.version.as_str()
            ))
            .font(crate::fonts::semibold(15.0))
            .color(kit.color(Tok::TextPrimary)),
        );
        if !m.plugin.description.is_empty() {
            ui.label(
                RichText::new(&m.plugin.description)
                    .size(12.0)
                    .color(kit.color(Tok::TextSecondary)),
            );
        }
        ui.add_space(10.0);
        for (key, granted) in m.permissions.rows() {
            let mut text = t!(key);
            if key == "plugins:perm.network" && granted && !m.permissions.network_hosts.is_empty() {
                text.push_str(&format!(" ({})", m.permissions.network_hosts.join(", ")));
            }
            permission_row(ui, kit, granted, &text, 13.0);
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let confirm = if r.install {
                t!("plugins:review.install")
            } else {
                t!("plugins:review.enable")
            };
            if ui.button(confirm).clicked() {
                cmd(actions, "plugins.reviewAnswer", json!({ "approve": true }));
            }
            if ui.button(t!("common:cancel")).clicked() {
                cmd(actions, "plugins.reviewAnswer", json!({ "approve": false }));
            }
        });
    });
}

/// `"Word Count" wants to run: git status` — Allow Once / Always / Deny.
pub fn process_prompt(ctx: &egui::Context, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    if core.plugins.review.is_some() {
        return;
    }
    let Some(p) = core.plugins.process_prompts.front() else {
        return;
    };
    let name = core
        .plugins
        .entry(&p.plugin)
        .map(|e| e.name())
        .unwrap_or_default();
    egui::Modal::new(egui::Id::new("plugin-process")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(
            RichText::new(t!("plugins:process.title", name = name.as_str()))
                .font(crate::fonts::semibold(15.0))
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(&p.command_line)
                .font(crate::fonts::mono(12.0))
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("plugins:process.once")).clicked() {
                cmd(
                    actions,
                    "plugins.processAnswer",
                    json!({ "answer": "once" }),
                );
            }
            if ui.button(t!("plugins:process.always")).clicked() {
                cmd(
                    actions,
                    "plugins.processAnswer",
                    json!({ "answer": "always" }),
                );
            }
            if ui.button(t!("plugins:process.deny")).clicked() {
                cmd(
                    actions,
                    "plugins.processAnswer",
                    json!({ "answer": "deny" }),
                );
            }
        });
    });
}

// ── status items ─────────────────────────────────────────────────────────

/// Plugin status items for one side, highest priority first.
pub fn status_items(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    right: bool,
    actions: &mut Actions,
) {
    let mut items: Vec<&crate::plugin_contrib::PluginStatusItem> = core
        .plugins
        .status_items
        .iter()
        .filter(|i| i.text.is_some() && (i.alignment == "right") == right)
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.priority));
    for item in items {
        let text = RichText::new(item.text.as_deref().unwrap_or_default())
            .size(12.0)
            .color(kit.color(Tok::TextSecondary));
        let mut resp = ui.add(egui::Button::new(text).frame(false));
        if let Some(tip) = &item.tooltip {
            resp = resp.on_hover_text(tip);
        }
        if resp.clicked()
            && let Some(command) = &item.command
        {
            cmd(
                actions,
                "plugins.panelAction",
                json!({ "plugin": item.plugin, "command": command }),
            );
        }
        ui.add_space(6.0);
    }
}

// ── declarative panels ───────────────────────────────────────────────────

/// The Plugins panel: one section per contributed panel.
pub fn plugins_panel(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    if core.plugins.panels.is_empty() {
        ui.label(
            RichText::new(t!("plugins:panels.empty"))
                .size(12.0)
                .color(kit.color(Tok::TextTertiary)),
        );
        if ui.button(t!("commands:plugins.manager")).clicked() {
            cmd(actions, "plugins.manager", json!({}));
        }
        return;
    }
    let panels = core.plugins.panels.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for p in panels {
                egui::CollapsingHeader::new(RichText::new(&p.title).size(12.0).strong())
                    .id_salt(("plugin-panel", &p.plugin, &p.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        let key = (p.plugin.clone(), p.id.clone());
                        match core.plugins.panel_trees.get(&key).cloned() {
                            Some(tree) => {
                                let prefix = format!("{}/{}", p.plugin, p.id);
                                node(ui, kit, core, &p.plugin, &tree, &prefix, actions)
                            }
                            None => {
                                ui.label(
                                    RichText::new(t!("plugins:panels.noContent"))
                                        .size(12.0)
                                        .color(kit.color(Tok::TextTertiary)),
                                );
                            }
                        }
                    });
            }
        });
}

fn action(actions: &mut Actions, plugin: &str, command: Option<&str>, args: Value) {
    if let Some(c) = command {
        cmd(
            actions,
            "plugins.panelAction",
            json!({ "plugin": plugin, "command": c, "args": args }),
        );
    }
}

fn str_of<'a>(n: &'a Value, k: &str) -> Option<&'a str> {
    n.get(k).and_then(Value::as_str)
}

/// One node of a declarative tree (column, row, label, heading, button,
/// checkbox, text_input, list, tree, separator, progress).
fn node(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    plugin: &str,
    n: &Value,
    path: &str,
    actions: &mut Actions,
) {
    let text = str_of(n, "text").unwrap_or_default();
    let children = |n: &Value| {
        n.get("children")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    match str_of(n, "type").unwrap_or("column") {
        "column" => {
            ui.vertical(|ui| {
                for (i, c) in children(n).iter().enumerate() {
                    node(ui, kit, core, plugin, c, &format!("{path}/{i}"), actions);
                }
            });
        }
        "row" => {
            ui.horizontal_wrapped(|ui| {
                for (i, c) in children(n).iter().enumerate() {
                    node(ui, kit, core, plugin, c, &format!("{path}/{i}"), actions);
                }
            });
        }
        "label" => {
            ui.label(
                RichText::new(text)
                    .size(12.0)
                    .color(kit.color(Tok::TextSecondary)),
            );
        }
        "heading" => {
            ui.label(
                RichText::new(text)
                    .font(crate::fonts::semibold(13.0))
                    .color(kit.color(Tok::TextPrimary)),
            );
        }
        "button" => {
            if ui.button(text).clicked() {
                action(
                    actions,
                    plugin,
                    str_of(n, "command"),
                    n.get("args").cloned().unwrap_or(Value::Null),
                );
            }
        }
        "checkbox" => {
            let mut checked = n.get("checked").and_then(Value::as_bool).unwrap_or(false);
            if ui.checkbox(&mut checked, text).changed() {
                action(
                    actions,
                    plugin,
                    str_of(n, "command"),
                    json!({ "checked": checked }),
                );
            }
        }
        "text_input" => {
            let key = format!("{path}:{}", str_of(n, "id").unwrap_or_default());
            let buffer = core
                .plugins
                .input_buffers
                .entry(key)
                .or_insert_with(|| str_of(n, "value").unwrap_or_default().to_owned());
            let resp = ui.add(
                egui::TextEdit::singleline(buffer)
                    .hint_text(str_of(n, "placeholder").unwrap_or_default())
                    .desired_width(f32::INFINITY),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let value = buffer.clone();
                action(
                    actions,
                    plugin,
                    str_of(n, "command"),
                    json!({ "value": value }),
                );
            }
        }
        "list" => {
            let on_select = str_of(n, "on_select");
            for (i, item) in n
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let label = item
                    .as_str()
                    .or_else(|| str_of(item, "text"))
                    .unwrap_or_default();
                let id = item.get("id").cloned().unwrap_or(json!(i + 1));
                if ui.selectable_label(false, label).clicked() {
                    action(
                        actions,
                        plugin,
                        on_select,
                        json!({ "item": id, "text": label }),
                    );
                }
            }
        }
        "tree" => {
            let on_select = str_of(n, "on_select");
            tree_items(ui, n.get("items"), plugin, on_select, path, actions);
        }
        "separator" => {
            ui.separator();
        }
        "progress" => {
            let v = n.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let mut bar = egui::ProgressBar::new(v.clamp(0.0, 1.0));
            if !text.is_empty() {
                bar = bar.text(text);
            }
            ui.add(bar);
        }
        other => {
            ui.label(
                RichText::new(t!("plugins:panels.unknownNode", kind = other))
                    .size(11.0)
                    .color(kit.color(Tok::Warning)),
            );
        }
    }
}

fn tree_items(
    ui: &mut egui::Ui,
    items: Option<&Value>,
    plugin: &str,
    on_select: Option<&str>,
    path: &str,
    actions: &mut Actions,
) {
    for (i, item) in items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let label = str_of(item, "text").unwrap_or_default();
        let id = item.get("id").cloned().unwrap_or(json!(i + 1));
        let kids = item
            .get("children")
            .filter(|c| c.as_array().is_some_and(|a| !a.is_empty()));
        match kids {
            Some(kids) => {
                let resp = egui::CollapsingHeader::new(label)
                    .id_salt((path, i))
                    .show(ui, |ui| {
                        tree_items(
                            ui,
                            Some(kids),
                            plugin,
                            on_select,
                            &format!("{path}/{i}"),
                            actions,
                        )
                    });
                if resp.header_response.double_clicked() {
                    action(
                        actions,
                        plugin,
                        on_select,
                        json!({ "item": id, "text": label }),
                    );
                }
            }
            None => {
                if ui.selectable_label(false, label).clicked() {
                    action(
                        actions,
                        plugin,
                        on_select,
                        json!({ "item": id, "text": label }),
                    );
                }
            }
        }
    }
}

// ── Plugin Manager ───────────────────────────────────────────────────────

pub fn manager(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    use crate::plugin_contrib::ManagerTab;
    use crate::plugins::PluginStatus;
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::Vec2::new(20.0, 12.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    let ui = &mut ui;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(t!("plugins:manager.title"))
                .font(crate::fonts::semibold(15.0))
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(&mut core.plugins.manager.search)
                .hint_text(t!("plugins:manager.search"))
                .desired_width(220.0),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("commands:plugins.installFromZip")).clicked() {
                cmd(actions, "plugins.installFromZip", json!({}));
            }
            if ui
                .button(t!("commands:plugins.installFromFolder"))
                .clicked()
            {
                cmd(actions, "plugins.installFromFolder", json!({}));
            }
        });
    });
    ui.add_space(8.0);
    let installed = core.plugins.entries.len();
    let recommended = core.plugins.recommended.len();
    ui.horizontal(|ui| {
        let tab = &mut core.plugins.manager.tab;
        ui.selectable_value(
            tab,
            ManagerTab::Installed,
            t!("plugins:manager.installed", count = installed),
        );
        ui.selectable_value(
            tab,
            ManagerTab::Recommended,
            t!("plugins:manager.recommended", count = recommended),
        );
    });
    ui.separator();
    let query = core.plugins.manager.search.to_lowercase();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if core.plugins.manager.tab == ManagerTab::Recommended {
                if core.plugins.recommended.is_empty() {
                    ui.label(
                        RichText::new(t!("plugins:manager.noRecommendations"))
                            .size(12.0)
                            .color(kit.color(Tok::TextTertiary)),
                    );
                }
                for (id, version) in core.plugins.recommended.clone() {
                    let state = match core.plugins.entry(&id) {
                        Some(e) => t!(AppCore::plugin_status_label(&e.status)),
                        None => t!("plugins:manager.notInstalled"),
                    };
                    ui.label(
                        RichText::new(format!("{id}  {version}  —  {state}"))
                            .size(13.0)
                            .color(kit.color(Tok::TextPrimary)),
                    );
                }
                return;
            }
            if core.plugins.entries.is_empty() {
                ui.label(
                    RichText::new(t!("plugins:manager.empty"))
                        .size(12.0)
                        .color(kit.color(Tok::TextTertiary)),
                );
            }
            let ids: Vec<String> = core.plugins.entries.iter().map(|e| e.id.clone()).collect();
            for id in ids {
                let Some(e) = core.plugins.entry(&id) else {
                    continue;
                };
                let name = e.name();
                let version = e
                    .manifest
                    .as_ref()
                    .map(|m| m.plugin.version.clone())
                    .unwrap_or_default();
                let description = e
                    .manifest
                    .as_ref()
                    .map(|m| m.plugin.description.clone())
                    .unwrap_or_default();
                if !query.is_empty()
                    && !name.to_lowercase().contains(&query)
                    && !id.contains(&query)
                    && !description.to_lowercase().contains(&query)
                {
                    continue;
                }
                let status = e.status.clone();
                let perms = e.manifest.as_ref().map(|m| m.permissions.clone());
                let has_settings = e
                    .manifest
                    .as_ref()
                    .is_some_and(|m| !m.contributes.settings.is_empty());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{name}  {version}"))
                            .font(crate::fonts::semibold(14.0))
                            .color(kit.color(Tok::TextPrimary)),
                    );
                    let color = match status {
                        PluginStatus::Enabled | PluginStatus::Active => kit.color(Tok::Accent),
                        PluginStatus::Invalid(_) | PluginStatus::Incompatible(_) => {
                            kit.color(Tok::Danger)
                        }
                        PluginStatus::NeedsReview { .. } => kit.color(Tok::Warning),
                        PluginStatus::Disabled => kit.color(Tok::TextTertiary),
                    };
                    ui.label(
                        RichText::new(t!(AppCore::plugin_status_label(&status)))
                            .size(12.0)
                            .color(color),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let args = json!({ "plugin": id });
                        if ui.small_button(t!("plugins:manager.uninstall")).clicked() {
                            cmd(actions, "plugins.uninstall", args.clone());
                        }
                        if perms.is_some() {
                            if has_settings
                                && ui.small_button(t!("plugins:manager.settings")).clicked()
                            {
                                cmd(actions, "plugins.settings", args.clone());
                            }
                            if ui.small_button(t!("plugins:manager.permissions")).clicked() {
                                cmd(actions, "plugins.togglePermissions", args.clone());
                            }
                        }
                        let toggle = match status {
                            PluginStatus::Enabled | PluginStatus::Active => {
                                Some(("plugins:manager.disable", "plugins.disable"))
                            }
                            PluginStatus::Disabled => {
                                Some(("plugins:manager.enable", "plugins.enable"))
                            }
                            PluginStatus::NeedsReview { .. } => {
                                Some(("plugins:reviewAction", "plugins.review"))
                            }
                            _ => None,
                        };
                        if let Some((label, id)) = toggle
                            && ui.small_button(t!(label)).clicked()
                        {
                            cmd(actions, id, args.clone());
                        }
                    });
                });
                match &status {
                    PluginStatus::Invalid(err) | PluginStatus::Incompatible(err) => {
                        ui.label(
                            RichText::new(err)
                                .font(crate::fonts::mono(12.0))
                                .color(kit.color(Tok::Danger)),
                        );
                    }
                    _ => {
                        if !description.is_empty() {
                            ui.label(
                                RichText::new(&description)
                                    .size(12.0)
                                    .color(kit.color(Tok::TextSecondary)),
                            );
                        }
                    }
                }
                if let Some(p) = &perms {
                    let granted: Vec<String> = p
                        .rows()
                        .into_iter()
                        .filter(|(_, on)| *on)
                        .map(|(k, _)| t!(k))
                        .collect();
                    let summary = if granted.is_empty() {
                        t!("plugins:manager.noPermissions")
                    } else {
                        t!("plugins:manager.permissionsList", list = granted.join(", "))
                    };
                    ui.label(
                        RichText::new(summary)
                            .size(11.0)
                            .color(kit.color(Tok::TextTertiary)),
                    );
                    if core.plugins.manager.expanded.as_deref() == Some(id.as_str()) {
                        for (k, on) in p.rows() {
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                permission_row(ui, kit, on, &t!(k), 12.0);
                            });
                        }
                        let always = core.mcp.gate.decision(
                            &notoir_core::permissions::Principal::Plugin(id.clone()),
                            &notoir_core::permissions::Capability::Process,
                        ) == notoir_core::permissions::Decision::Allow;
                        if p.process && always {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(t!("plugins:manager.processAlways")).size(12.0),
                                );
                                if ui.small_button(t!("plugins:manager.revoke")).clicked() {
                                    cmd(actions, "plugins.revokeProcess", json!({ "plugin": id }));
                                }
                            });
                        }
                    }
                }
                ui.separator();
            }
        });
}

/// Uninstall confirmation.
pub fn uninstall_confirm(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &AppCore,
    actions: &mut Actions,
) {
    let Some(id) = &core.plugins.uninstall_confirm else {
        return;
    };
    let name = core
        .plugins
        .entry(id)
        .map(|e| e.name())
        .unwrap_or_else(|| id.clone());
    egui::Modal::new(egui::Id::new("plugin-uninstall")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(
            RichText::new(t!("plugins:uninstall.title", name = name.as_str()))
                .font(crate::fonts::semibold(15.0))
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.label(
            RichText::new(t!("plugins:uninstall.body"))
                .size(12.0)
                .color(kit.color(Tok::TextSecondary)),
        );
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button(
                    RichText::new(t!("plugins:manager.uninstall")).color(kit.color(Tok::Danger)),
                )
                .clicked()
            {
                cmd(
                    actions,
                    "plugins.uninstallAnswer",
                    json!({ "approve": true }),
                );
            }
            if ui.button(t!("common:cancel")).clicked() {
                cmd(
                    actions,
                    "plugins.uninstallAnswer",
                    json!({ "approve": false }),
                );
            }
        });
    });
}

// ── Developer Tools ──────────────────────────────────────────────────────

/// Plugin Developer Tools (the Plugin Output panel): per-plugin details and
/// the log.
pub fn dev_tools(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    let selected = core.plugins.dev_plugin.clone();
    ui.horizontal(|ui| {
        let label = selected
            .as_ref()
            .and_then(|id| core.plugins.entry(id).map(|e| e.name()))
            .unwrap_or_else(|| t!("plugins:dev.all"));
        egui::ComboBox::from_id_salt("plugin-dev-select")
            .selected_text(label)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(selected.is_none(), t!("plugins:dev.all"))
                    .clicked()
                {
                    core.plugins.dev_plugin = None;
                }
                for e in &core.plugins.entries {
                    if ui
                        .selectable_label(selected.as_deref() == Some(e.id.as_str()), e.name())
                        .clicked()
                    {
                        core.plugins.dev_plugin = Some(e.id.clone());
                    }
                }
            });
        if let Some(id) = &selected
            && ui.button(t!("plugins:dev.reload")).clicked()
        {
            cmd(actions, "plugins.reloadPlugin", json!({ "plugin": id }));
        }
    });
    if let Some(e) = selected.as_ref().and_then(|id| core.plugins.entry(id)) {
        let mut lines = vec![t!(
            "plugins:dev.status",
            status = t!(AppCore::plugin_status_label(&e.status))
        )];
        if let Some(by) = &e.activated_by {
            lines.push(t!("plugins:dev.activatedBy", event = by.as_str()));
        }
        if e.manifest
            .as_ref()
            .is_some_and(|m| m.activates_on("onStartup"))
        {
            lines.push(t!("plugins:dev.onStartup"));
        }
        if !e.api_calls.is_empty() {
            let calls: Vec<String> = e
                .api_calls
                .iter()
                .map(|(k, n)| format!("{k} ×{n}"))
                .collect();
            lines.push(t!("plugins:dev.apiCalls", list = calls.join(", ")));
        }
        if let Some(err) = &e.last_error {
            lines.push(t!("plugins:dev.lastError", error = err.as_str()));
        }
        if !e.missing.is_empty() {
            let keys: Vec<&str> = e.missing.iter().map(String::as_str).collect();
            lines.push(t!("plugins:dev.missing", list = keys.join(", ")));
        }
        for l in lines {
            ui.label(
                RichText::new(l)
                    .size(12.0)
                    .color(kit.color(Tok::TextSecondary)),
            );
        }
        ui.separator();
    }
    let filter = selected.clone();
    let rows: Vec<&(String, String, String)> = core
        .plugins
        .output
        .iter()
        .filter(|(p, _, _)| filter.as_ref().is_none_or(|f| f == p))
        .collect();
    if rows.is_empty() {
        ui.label(
            RichText::new(t!("plugins:output.empty"))
                .size(12.0)
                .color(kit.color(Tok::TextTertiary)),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (plugin, level, message) in rows {
                let color = match level.as_str() {
                    "error" => kit.color(Tok::Danger),
                    "warn" => kit.color(Tok::Warning),
                    _ => kit.color(Tok::TextSecondary),
                };
                ui.label(
                    RichText::new(format!("[{plugin}] {message}"))
                        .font(crate::fonts::mono(12.0))
                        .color(color),
                );
            }
        });
}
