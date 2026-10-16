//! Notes UI (docs/specs/34): the Backlinks and Tags panels and the `[[`
//! completion popup.

use egui::{Align2, RichText, Sense};
use notoir_core::command::{CommandInvocation, InvocationSource};
use serde_json::{Value, json};

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::editor_widget::EditorCaches;
use crate::t;
use crate::theme::Tok;

fn cmd(actions: &mut Actions, id: &'static str, args: Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

fn hint(ui: &mut egui::Ui, kit: &Kit, text: String) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(kit.color(Tok::TextTertiary)),
    );
}

/// One clickable `note — "…context…"` row.
fn row(
    ui: &mut egui::Ui,
    kit: &Kit,
    note: &str,
    line: usize,
    snippet: &str,
    actions: &mut Actions,
) {
    let resp = ui
        .vertical(|ui| {
            ui.label(
                RichText::new(notoir_workspace::links::stem(note))
                    .size(13.0)
                    .color(kit.color(Tok::TextPrimary)),
            );
            if !snippet.is_empty() {
                ui.label(
                    RichText::new(snippet)
                        .size(11.0)
                        .color(kit.color(Tok::TextSecondary)),
                );
            }
        })
        .response
        .interact(Sense::click())
        .on_hover_text(note);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        cmd(
            actions,
            "notes.openLine",
            json!({ "note": note, "line": line }),
        );
    }
    ui.add_space(4.0);
}

/// Backlinks: notes linking to the active one, and unlinked mentions.
pub fn backlinks(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    let Some(rel) = core.notes_active_rel() else {
        hint(ui, kit, t!("notes:backlinks.noNote"));
        return;
    };
    if !core.notes.ready {
        hint(ui, kit, t!("notes:indexing"));
        return;
    }
    let links = core.notes.index.backlinks(&rel);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(
                RichText::new(t!("notes:backlinks.referencedBy", count = links.len()))
                    .size(12.0)
                    .color(kit.color(Tok::TextSecondary)),
            );
            ui.add_space(4.0);
            if links.is_empty() {
                hint(ui, kit, t!("notes:backlinks.none"));
            }
            for b in &links {
                row(ui, kit, &b.source, b.line, &b.snippet, actions);
            }
            ui.add_space(8.0);
            let computed = core
                .notes
                .unlinked
                .as_ref()
                .filter(|(r, _)| *r == rel)
                .map(|(_, f)| f.clone());
            let header = match &computed {
                Some(f) => t!("notes:backlinks.unlinkedCount", count = f.len()),
                None => t!("notes:backlinks.unlinked"),
            };
            let resp = egui::CollapsingHeader::new(RichText::new(header).size(12.0))
                .id_salt(("unlinked", &rel))
                .default_open(false)
                .show(ui, |ui| match &computed {
                    Some(found) if found.is_empty() => {
                        hint(ui, kit, t!("notes:backlinks.noMentions"))
                    }
                    Some(found) => {
                        for m in found {
                            row(ui, kit, &m.source, m.line, &m.snippet, actions);
                        }
                    }
                    None => hint(ui, kit, t!("notes:searching")),
                });
            if resp.body_returned.is_some() && computed.is_none() {
                cmd(actions, "notes.findUnlinked", json!({ "note": rel }));
            }
        });
}

/// Tags with counts; choosing one lists its notes below.
pub fn tags(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    if core.workspace.is_none() {
        hint(ui, kit, t!("notes:tags.noWorkspace"));
        return;
    }
    if !core.notes.ready {
        hint(ui, kit, t!("notes:indexing"));
        return;
    }
    let all = core.notes.index.tags();
    if all.is_empty() {
        hint(ui, kit, t!("notes:tags.none"));
        return;
    }
    let selected = core.notes.tag_filter.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (tag, n) in &all {
                    let on = selected
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(tag));
                    let label = format!("#{tag}  {n}");
                    if ui.selectable_label(on, label).clicked() {
                        let next = if on { Value::Null } else { json!(tag) };
                        cmd(actions, "notes.filterTag", json!({ "tag": next }));
                    }
                }
            });
            if let Some(tag) = &selected {
                ui.separator();
                for rel in core.notes.index.files_with_tag(tag) {
                    row(ui, kit, &rel, 0, "", actions);
                }
            }
        });
}

/// The `[[` completion list under the caret.
pub fn completion(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &AppCore,
    editors: &EditorCaches,
    actions: &mut Actions,
) {
    let Some(c) = &core.notes.completion else {
        return;
    };
    let Some(caret) = editors.caret_rect(c.view) else {
        return;
    };
    egui::Area::new(egui::Id::new("wiki-completion"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(caret.left_bottom() + egui::vec2(0.0, 2.0))
        .constrain_to(ctx.content_rect())
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(kit.color(Tok::BgPanelRaised))
                .show(ui, |ui| {
                    ui.set_min_width(220.0);
                    for (i, (name, rel)) in c.items.iter().enumerate() {
                        let resp = ui
                            .selectable_label(i == c.selected, RichText::new(name).size(13.0))
                            .on_hover_text(rel);
                        if resp.clicked() {
                            cmd(actions, "notes.complete", json!({ "index": i }));
                        }
                    }
                });
        });
}
