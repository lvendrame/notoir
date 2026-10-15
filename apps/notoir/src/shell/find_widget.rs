//! The find / replace widget docked at the top right of an editor view
//! (docs/specs/11 §Find / Replace).

use egui::{Key, Pos2, Rect, RichText, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::ViewId;

use super::{Actions, Kit, icon_button_sized};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::t;
use crate::theme::Tok;

const WIDTH: f32 = 460.0;

fn cmd(actions: &mut Actions, id: &'static str) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal));
}

fn toggle(ui: &mut egui::Ui, kit: &Kit, text: &str, on: bool, tip: &str) -> bool {
    let fill = if on {
        kit.color(Tok::AccentSoft)
    } else {
        egui::Color32::TRANSPARENT
    };
    let color = if on {
        kit.color(Tok::Accent)
    } else {
        kit.color(Tok::TextSecondary)
    };
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .font(crate::fonts::mono(12.0))
                .color(color),
        )
        .fill(fill)
        .min_size(Vec2::new(24.0, 22.0)),
    )
    .on_hover_text(tip)
    .clicked()
}

/// Draws the widget for `view` inside `body` when it is open.
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    view: ViewId,
    body: Rect,
    actions: &mut Actions,
) {
    if !core.find.widgets.get(&view).is_some_and(|w| w.open) {
        return;
    }
    core.refresh_find(view, false);
    let w = core.find.widgets.get(&view).cloned().expect("open");
    let opts = core.find.options;
    let width = WIDTH.min(body.width() - 24.0);
    let height = if w.replace_open { 66.0 } else { 36.0 };
    let rect = Rect::from_min_size(
        Pos2::new(body.right() - width - 24.0, body.top() + 4.0),
        Vec2::new(width, height),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let painter = child.painter();
    painter.rect_filled(rect, 4.0, kit.color(Tok::BgOverlay));
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, kit.color(Tok::BorderStrong)),
        egui::StrokeKind::Inside,
    );
    // Claim the area so clicks do not reach the editor below.
    child.interact(
        rect,
        egui::Id::new(("find-widget", view)),
        egui::Sense::click(),
    );
    let mut query = w.query.clone();
    let mut replacement = w.replacement.clone();
    let mut query_resp = None;
    let inner = rect.shrink2(Vec2::new(6.0, 5.0));
    let mut rows = child.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    rows.spacing_mut().item_spacing = Vec2::new(3.0, 4.0);
    rows.horizontal(|ui| {
        let chevron = if w.replace_open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        if icon_button_sized(
            ui,
            kit,
            chevron,
            &t!("find:toggleReplace"),
            false,
            true,
            22.0,
        )
        .clicked()
            && let Some(x) = core.find.widgets.get_mut(&view)
        {
            x.replace_open = !x.replace_open;
        }
        let resp = ui.add(
            egui::TextEdit::singleline(&mut query)
                .id(egui::Id::new(("find-query", view)))
                .return_key(None)
                .hint_text(t!("find:hint"))
                .desired_width(170.0),
        );
        if w.error.is_some() {
            ui.painter().rect_stroke(
                resp.rect,
                2.0,
                Stroke::new(1.0, kit.color(Tok::Danger)),
                egui::StrokeKind::Outside,
            );
        }
        if toggle(
            ui,
            kit,
            "Aa",
            opts.case_sensitive,
            &t!("find:caseSensitive"),
        ) {
            cmd(actions, "edit.toggleFindCaseSensitive");
        }
        if toggle(ui, kit, "ab", opts.whole_word, &t!("find:wholeWord")) {
            cmd(actions, "edit.toggleFindWholeWord");
        }
        if toggle(ui, kit, ".*", opts.regex, &t!("find:regex")) {
            cmd(actions, "edit.toggleFindRegex");
        }
        let status = if let Some(e) = &w.error {
            RichText::new(e).color(kit.color(Tok::Danger))
        } else if w.wrapped_until.is_some_and(|t| t > core.now_ms) {
            RichText::new(t!("find:wrapped")).color(kit.color(Tok::TextSecondary))
        } else if w.query.is_empty() {
            RichText::new("")
        } else if w.matches.is_empty() {
            RichText::new(t!("find:noResults")).color(kit.color(Tok::Danger))
        } else {
            let cur = w
                .current
                .map_or_else(|| "?".to_owned(), |c| (c + 1).to_string());
            RichText::new(t!("find:count", current = cur, total = w.matches.len()))
                .color(kit.color(Tok::TextSecondary))
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button_sized(ui, kit, Icon::X, &t!("find:close"), false, true, 22.0).clicked() {
                cmd(actions, "edit.closeFind");
            }
            if toggle(ui, kit, "≡", w.within.is_some(), &t!("find:inSelection")) {
                cmd(actions, "edit.toggleFindInSelection");
            }
            let has = !w.matches.is_empty();
            if icon_button_sized(
                ui,
                kit,
                Icon::ChevronDown,
                &t!("find:next"),
                false,
                has,
                22.0,
            )
            .clicked()
            {
                cmd(actions, "edit.findNext");
            }
            if icon_button_sized(
                ui,
                kit,
                Icon::ChevronUp,
                &t!("find:previous"),
                false,
                has,
                22.0,
            )
            .clicked()
            {
                cmd(actions, "edit.findPrevious");
            }
            // The status takes the space left, left-aligned.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(status.size(12.0)).truncate());
            });
        });
        query_resp = Some(resp);
    });
    let mut replace_resp = None;
    if w.replace_open {
        rows.horizontal(|ui| {
            ui.add_space(25.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut replacement)
                    .id(egui::Id::new(("find-replace", view)))
                    .return_key(None)
                    .hint_text(t!("find:replaceHint"))
                    .desired_width(170.0),
            );
            let enabled = !w.matches.is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new(t!("find:replace")))
                .clicked()
            {
                cmd(actions, "edit.replaceOne");
            }
            if ui
                .add_enabled(enabled, egui::Button::new(t!("find:replaceAll")))
                .clicked()
            {
                cmd(actions, "edit.replaceAll");
            }
            replace_resp = Some(resp);
        });
    }
    let query_resp = query_resp.expect("drawn");
    if w.focus {
        query_resp.request_focus();
        if let Some(x) = core.find.widgets.get_mut(&view) {
            x.focus = false;
        }
    }
    // Keys while the widget has focus.
    // Esc makes egui drop the field's focus first, so a field that just
    // lost focus still gets its keys.
    let focused = query_resp.has_focus()
        || query_resp.lost_focus()
        || replace_resp
            .as_ref()
            .is_some_and(|r| r.has_focus() || r.lost_focus());
    if focused {
        let (enter, shift_enter, esc, alt_c, alt_w, alt_r) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::SHIFT, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape) || i.key_pressed(Key::Escape),
                i.consume_key(egui::Modifiers::ALT, Key::C),
                i.consume_key(egui::Modifiers::ALT, Key::W),
                i.consume_key(egui::Modifiers::ALT, Key::R),
            )
        });
        let in_replace = replace_resp
            .as_ref()
            .is_some_and(|r| r.has_focus() || r.lost_focus());
        if enter && in_replace {
            cmd(actions, "edit.replaceOne");
        } else if enter {
            cmd(actions, "edit.findNext");
        }
        if shift_enter {
            cmd(actions, "edit.findPrevious");
        }
        if esc {
            cmd(actions, "edit.closeFind");
        }
        if alt_c {
            cmd(actions, "edit.toggleFindCaseSensitive");
        }
        if alt_w {
            cmd(actions, "edit.toggleFindWholeWord");
        }
        if alt_r {
            cmd(actions, "edit.toggleFindRegex");
        }
        // Keep focus in the field after Enter.
        if enter || shift_enter {
            if in_replace {
                if let Some(r) = &replace_resp {
                    r.request_focus();
                }
            } else {
                query_resp.request_focus();
            }
        }
    }
    let changed = query != w.query;
    if let Some(x) = core.find.widgets.get_mut(&view) {
        x.query = query;
        x.replacement = replacement;
    }
    if changed {
        core.refresh_find(view, true);
    }
}
