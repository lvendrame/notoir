//! The recovery prompt (docs/specs/09 §Startup detection): a non-modal panel
//! over the editor area listing unsaved documents from a crashed session,
//! with Restore All / Review… / Discard and per-entry actions in Review.

use egui::{Align, Layout, RichText, Stroke};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_storage::recovery::EntryStatus;
use serde_json::json;

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::t;
use crate::theme::Tok;

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

pub fn show(ctx: &egui::Context, kit: &mut Kit, core: &AppCore, top: f32, actions: &mut Actions) {
    let entries = core.recovery.prompt_entries();
    if entries.is_empty() {
        return;
    }
    let review = core.recovery.review;
    egui::Area::new(egui::Id::new("recovery-prompt"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top + 48.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(kit.color(Tok::BgPanelRaised))
                .stroke(Stroke::new(1.0, kit.color(Tok::BorderStrong)))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(520.0);
                    ui.label(
                        RichText::new(t!("recovery:title"))
                            .font(crate::fonts::semibold(15.0))
                            .color(kit.color(Tok::TextPrimary)),
                    );
                    ui.add_space(10.0);
                    for e in &entries {
                        let meta = e.meta.as_ref();
                        let title = meta.map_or_else(|| e.id.clone(), |m| m.title.clone());
                        let dir = meta
                            .and_then(|m| m.path.as_deref())
                            .and_then(|p| std::path::Path::new(p).parent())
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let detail = match &e.status {
                            EntryStatus::Damaged(_) => t!("recovery:damaged"),
                            EntryStatus::Ok => {
                                let lines =
                                    e.content.as_deref().map_or(0, |c| c.lines().count().max(1));
                                t!("recovery:lines", count = lines)
                            }
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&title)
                                    .size(13.0)
                                    .color(kit.color(Tok::TextPrimary)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if review {
                                    if ui.button(t!("recovery:discardOne")).clicked() {
                                        cmd(
                                            actions,
                                            "recovery.discardEntry",
                                            json!({ "id": e.id }),
                                        );
                                    }
                                    if ui.button(t!("recovery:restoreOne")).clicked() {
                                        cmd(
                                            actions,
                                            "recovery.restoreEntry",
                                            json!({ "id": e.id }),
                                        );
                                    }
                                }
                                ui.label(
                                    RichText::new(detail)
                                        .size(12.0)
                                        .color(kit.color(Tok::TextSecondary)),
                                );
                                ui.add_space(8.0);
                                // The folder takes what is left, elided.
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(dir)
                                                .size(12.0)
                                                .color(kit.color(Tok::TextTertiary)),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        });
                    }
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(
                                RichText::new(t!("recovery:discard")).color(kit.color(Tok::Danger)),
                            )
                            .clicked()
                        {
                            cmd(actions, "recovery.discard", json!({}));
                        }
                        let review_label = if review {
                            t!("recovery:hideReview")
                        } else {
                            t!("recovery:review")
                        };
                        if ui.button(review_label).clicked() {
                            cmd(actions, "recovery.review", json!({}));
                        }
                        if ui.button(t!("recovery:restoreAll")).clicked() {
                            cmd(actions, "recovery.restoreAll", json!({}));
                        }
                    });
                });
        });
}
