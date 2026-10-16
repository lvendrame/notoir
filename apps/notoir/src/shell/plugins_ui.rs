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
            let (mark, color) = if granted {
                ("✓", kit.color(Tok::TextPrimary))
            } else {
                ("✕", kit.color(Tok::TextTertiary))
            };
            let mut text = format!("{mark}  {}", t!(key));
            if key == "plugins:perm.network" && granted && !m.permissions.network_hosts.is_empty() {
                text.push_str(&format!(" ({})", m.permissions.network_hosts.join(", ")));
            }
            ui.label(RichText::new(text).size(13.0).color(color));
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

/// Plugin Output: every plugin's log, newest last.
pub fn output(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore) {
    if core.plugins.output.is_empty() {
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
            for (plugin, level, message) in &core.plugins.output {
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
