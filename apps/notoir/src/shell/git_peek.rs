//! Gutter peek (docs/specs/28 §Decorations): clicking a change bar shows the
//! original lines under the change with Revert Change and Stage Change.

use egui::{Align, Key, Layout, Pos2, Rect, RichText, Stroke};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::ViewId;

use super::{Actions, Kit, icon_button_sized};
use crate::core::AppCore;
use crate::editor_widget::EditorCaches;
use crate::icons::Icon;
use crate::t;
use crate::theme::Tok;

/// Base lines shown before the peek scrolls.
const MAX_ROWS: usize = 10;

fn cmd(actions: &mut Actions, id: &'static str) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal));
}

/// Draws the peek below the change when it belongs to `view`.
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    editors: &mut EditorCaches,
    view: ViewId,
    body: Rect,
    actions: &mut Actions,
) {
    let Some(p) = core.git.peek.as_ref().filter(|p| p.view == view) else {
        return;
    };
    // Below the last changed line (a deletion: where the lines were).
    let y = if p.buf.end == 0 {
        editors.line_y(view, 0, false).map(|(top, _)| top)
    } else {
        editors
            .line_y(view, p.buf.end - 1, false)
            .map(|(top, h)| top + h)
    };
    let Some(y) = y.filter(|y| *y >= body.top() && *y < body.bottom() - 20.0) else {
        return;
    };
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        cmd(actions, "git.closePeek");
        return;
    }
    let left = editors
        .geometry(view)
        .map_or(body.left(), |(_, gutter)| gutter.right());
    let width = (body.right() - left - 16.0).max(200.0);
    let lines: Vec<&str> = p
        .base_text
        .strip_suffix('\n')
        .unwrap_or(&p.base_text)
        .split('\n')
        .collect();
    let font = crate::fonts::mono(12.0);
    egui::Area::new(egui::Id::new(("git-peek", view.0)))
        .fixed_pos(Pos2::new(left, y))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(kit.color(Tok::BgPanelRaised))
                .stroke(Stroke::new(1.0, kit.color(Tok::Border)))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(width - 20.0);
                    ui.horizontal(|ui| {
                        let title = t!("git:peek.title", n = p.index + 1, total = p.total);
                        ui.label(
                            RichText::new(title)
                                .size(12.0)
                                .color(kit.color(Tok::TextSecondary)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button_sized(
                                ui,
                                kit,
                                Icon::X,
                                &t!("git:peek.close"),
                                false,
                                true,
                                22.0,
                            )
                            .clicked()
                            {
                                cmd(actions, "git.closePeek");
                            }
                            if p.stageable
                                && ui
                                    .add_enabled(
                                        !core.git_busy(),
                                        egui::Button::new(t!("commands:git.stageChange")),
                                    )
                                    .clicked()
                            {
                                cmd(actions, "git.stageChange");
                            }
                            if ui.button(t!("commands:git.revertChange")).clicked() {
                                cmd(actions, "git.revertChange");
                            }
                        });
                    });
                    if p.base_text.is_empty() {
                        ui.label(
                            RichText::new(t!("git:peek.added"))
                                .size(12.0)
                                .color(kit.color(Tok::TextTertiary)),
                        );
                        return;
                    }
                    let row_h = ui.fonts_mut(|f| f.row_height(&font));
                    egui::ScrollArea::vertical()
                        .max_height(row_h * MAX_ROWS as f32 + 4.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for line in &lines {
                                ui.label(
                                    RichText::new(format!("- {line}"))
                                        .font(font.clone())
                                        .color(kit.color(Tok::GitDeleted)),
                                );
                            }
                        });
                });
        });
}
