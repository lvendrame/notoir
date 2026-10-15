//! Structural minimap (docs/specs/13): one block per buffer line from its
//! indentation to its last non-blank column, a viewport rectangle, search and
//! selection marks. Click centres the editor on a line; dragging the viewport
//! scrolls.

use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_editor::display_map::line_text;
use serde_json::json;

use super::{EditorOptions, Metrics, ViewCache, scroll_y, set_scroll_y};
use crate::shell::Kit;
use crate::t;
use crate::theme::Tok;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapOptions {
    pub left: bool,
    pub width: f32,
    /// Logical px per buffer line.
    pub line_h: f32,
    pub opacity: f32,
    pub show_viewport: bool,
    pub show_search: bool,
    pub show_selection: bool,
}

/// Columns mapped across the minimap width.
const COLS: f32 = 100.0;

/// Vertical mapping between buffer lines and minimap pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapLayout {
    pub top: f32,
    pub height: f32,
    pub line_h: f32,
    /// First line drawn at `top` (the minimap scrolls with long documents).
    pub first_line: f32,
    pub total_lines: usize,
    /// Editor viewport in lines.
    pub view_top: f32,
    pub view_lines: f32,
}

impl MinimapLayout {
    pub fn new(
        top: f32,
        height: f32,
        line_h: f32,
        total_lines: usize,
        view_top: f32,
        view_lines: f32,
    ) -> Self {
        let fits = height / line_h;
        let total = total_lines as f32;
        // Long documents: the minimap scrolls proportionally so the viewport
        // indicator stays visible.
        let first_line = if total <= fits {
            0.0
        } else {
            let max_view_top = (total - view_lines).max(1.0);
            (view_top / max_view_top).clamp(0.0, 1.0) * (total - fits)
        };
        Self {
            top,
            height,
            line_h,
            first_line,
            total_lines,
            view_top,
            view_lines,
        }
    }

    pub fn line_y(&self, line: f32) -> f32 {
        self.top + (line - self.first_line) * self.line_h
    }

    /// Buffer line under a y coordinate.
    pub fn line_at(&self, y: f32) -> usize {
        let l = self.first_line + (y - self.top) / self.line_h;
        (l.max(0.0) as usize).min(self.total_lines.saturating_sub(1))
    }

    pub fn viewport(&self) -> (f32, f32) {
        let a = self.line_y(self.view_top);
        (a, a + self.view_lines * self.line_h)
    }

    /// Lines drawn (first, end exclusive).
    pub fn visible(&self) -> (usize, usize) {
        let first = self.first_line.floor() as usize;
        let n = (self.height / self.line_h).ceil() as usize + 1;
        (first, (first + n).min(self.total_lines))
    }
}

/// Paints and handles the minimap in `rect`. Commands from its context menu
/// go to `commands`.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &egui::Ui,
    kit: &mut Kit,
    id: egui::Id,
    rect: Rect,
    cache: &mut ViewCache,
    m: &Metrics,
    opts: &EditorOptions,
    mm: &MinimapOptions,
    selection: &notoir_editor::Selection,
    commands: &mut Vec<CommandInvocation>,
) {
    let total_lines = cache.rope.len_lines();
    let sy = scroll_y(cache, m);
    let top_row = sy / m.row_h;
    let view_top = cache.map.row_to_line(top_row.floor() as usize).line as f32;
    let view_lines = (m.text.height() / m.row_h).max(1.0);
    let layout = MinimapLayout::new(
        rect.top(),
        rect.height(),
        mm.line_h,
        total_lines,
        view_top,
        view_lines,
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, kit.color(Tok::EditorBackground));
    let block = kit.color(Tok::MinimapBlock).gamma_multiply(mm.opacity);
    let col_w = (rect.width() - 8.0) / COLS;
    let (first, end) = layout.visible();
    let tab = opts.indent.tab_size;
    for line in first..end {
        let text = line_text(&cache.rope, line);
        let mut col = 0usize;
        let mut indent = None;
        let mut last = 0usize;
        for c in text.chars() {
            let w = notoir_editor::wrap::char_cols(c, col, tab);
            if !c.is_whitespace() {
                indent.get_or_insert(col);
                last = col + w;
            }
            col += w;
        }
        let Some(start) = indent else { continue };
        let y = layout.line_y(line as f32);
        let x0 = rect.left() + 4.0 + start as f32 * col_w;
        let x1 = (rect.left() + 4.0 + last as f32 * col_w).min(rect.right() - 4.0);
        if x1 > x0 {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x0, y),
                    Pos2::new(x1, y + (mm.line_h - 0.5).max(0.5)),
                ),
                0.0,
                block,
            );
        }
    }
    // Selection marks.
    if mm.show_selection {
        for r in selection.ranges().iter().filter(|r| !r.is_empty()) {
            let a = cache.rope.char_to_line(r.from()) as f32;
            let b = cache.rope.char_to_line(r.to()) as f32 + 1.0;
            let rr = Rect::from_min_max(
                Pos2::new(rect.left(), layout.line_y(a)),
                Pos2::new(rect.right(), layout.line_y(b)),
            );
            painter.rect_filled(rr, 0.0, kit.color(Tok::EditorSelection).gamma_multiply(0.6));
        }
    }
    // Search marks: full-width accent ticks.
    if mm.show_search {
        let lines: Vec<usize> = opts
            .find_matches
            .iter()
            .map(|(a, _)| cache.rope.char_to_line(*a))
            .collect();
        for y in search_mark_ys(&layout, &lines) {
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), 1.0)),
                0.0,
                kit.color(Tok::Accent).gamma_multiply(0.8),
            );
        }
    }
    // Interaction.
    let resp = ui.interact(rect, id.with("minimap"), Sense::click_and_drag());
    let (vy0, vy1) = layout.viewport();
    let viewport = Rect::from_min_max(Pos2::new(rect.left(), vy0), Pos2::new(rect.right(), vy1));
    if mm.show_viewport {
        let color = if resp.hovered() || resp.dragged() {
            kit.color(Tok::MinimapViewportHover)
        } else {
            kit.color(Tok::MinimapViewport)
        };
        painter.rect_filled(viewport, 0.0, color);
    }
    painter.line_segment(
        if mm.left {
            [rect.right_top(), rect.right_bottom()]
        } else {
            [rect.left_top(), rect.left_bottom()]
        },
        Stroke::new(1.0, kit.color(Tok::Border)),
    );
    let center_on = |cache: &mut ViewCache, line: usize| {
        let row = cache.map.line_to_row(line) as f32;
        set_scroll_y(
            cache,
            m,
            row * m.row_h - m.text.height() / 2.0 + m.row_h / 2.0,
        );
    };
    if resp.drag_started()
        && let Some(p) = resp.interact_pointer_pos()
        && !viewport.contains(p)
    {
        // Jump, then drag from there.
        center_on(cache, layout.line_at(p.y));
    } else if resp.dragged() {
        let dy = resp.drag_delta().y;
        if dy != 0.0 {
            // The viewport follows the pointer: one minimap line per
            // `line_h` px, scaled when the minimap itself scrolls.
            let fits = layout.height / layout.line_h;
            let scale = if (total_lines as f32) <= fits {
                1.0
            } else {
                let max_top = (total_lines as f32 - view_lines).max(1.0);
                max_top / (layout.height - view_lines * layout.line_h).max(1.0) * layout.line_h
            };
            let lines = dy / layout.line_h * scale;
            let y = scroll_y(cache, m) + lines * m.row_h;
            set_scroll_y(cache, m, y);
        }
    } else if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        center_on(cache, layout.line_at(p.y));
    }
    if let Some(p) = resp.hover_pos() {
        let a = layout.line_at(p.y) + 1;
        let b = (a + (4.0 / mm.line_h).ceil() as usize).min(total_lines);
        resp.clone()
            .on_hover_text_at_pointer(t!("editor:minimapLines", from = a, to = b.max(a)));
    }
    resp.context_menu(|ui| {
        if ui.button(t!("editor:minimapHide")).clicked() {
            commands.push(CommandInvocation::new(
                "view.toggleMinimap",
                InvocationSource::ContextMenu,
            ));
            ui.close();
        }
        ui.menu_button(t!("editor:minimapPosition"), |ui| {
            if ui.radio(mm.left, t!("editor:minimapLeft")).clicked() {
                commands.push(
                    CommandInvocation::new("view.minimapSide", InvocationSource::ContextMenu)
                        .with_args(json!({ "side": "left" })),
                );
                ui.close();
            }
            if ui.radio(!mm.left, t!("editor:minimapRight")).clicked() {
                commands.push(
                    CommandInvocation::new("view.minimapSide", InvocationSource::ContextMenu)
                        .with_args(json!({ "side": "right" })),
                );
                ui.close();
            }
        });
        ui.menu_button(t!("editor:minimapScale"), |ui| {
            for s in 1..=3 {
                if ui
                    .radio((mm.line_h - s as f32).abs() < 0.01, format!("{s}×"))
                    .clicked()
                {
                    commands.push(
                        CommandInvocation::new("view.minimapScale", InvocationSource::ContextMenu)
                            .with_args(json!({ "scale": s })),
                    );
                    ui.close();
                }
            }
        });
    });
    // Wheel over the minimap scrolls the editor like the text area.
    if resp.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            let y = scroll_y(cache, m) - dy;
            set_scroll_y(cache, m, y);
        }
    }
}

/// y positions of search ticks inside the minimap (one per pixel row).
pub fn search_mark_ys(layout: &MinimapLayout, lines: &[usize]) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::new();
    for &l in lines {
        let y = layout.line_y(l as f32);
        if y < layout.top - 2.0 || y > layout.top + layout.height {
            continue;
        }
        if out.last().is_some_and(|last| (y - last).abs() < 1.0) {
            continue;
        }
        out.push(y);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec 13: "Search marks"
    #[test]
    fn one_tick_per_match_line() {
        let l = MinimapLayout::new(0.0, 600.0, 2.0, 300, 0.0, 40.0);
        let ys = search_mark_ys(&l, &[10, 150, 290]);
        assert_eq!(ys, vec![20.0, 300.0, 580.0]);
        // Matches on one line share a tick.
        assert_eq!(search_mark_ys(&l, &[10, 10, 11]).len(), 2);
    }

    #[test]
    fn short_documents_do_not_scroll() {
        let l = MinimapLayout::new(0.0, 600.0, 2.0, 100, 10.0, 30.0);
        assert_eq!(l.first_line, 0.0);
        assert_eq!(l.line_at(40.0), 20);
        assert_eq!(l.viewport(), (20.0, 80.0));
    }

    #[test]
    fn long_documents_keep_the_viewport_visible() {
        for top in [0.0, 500.0, 1960.0] {
            let l = MinimapLayout::new(0.0, 600.0, 2.0, 2000, top, 40.0);
            let (a, b) = l.viewport();
            assert!(a >= -0.01 && b <= 600.01, "top {top}: {a}..{b}");
            assert_eq!(l.line_at(l.line_y(1500.0) + 0.5), 1500);
        }
    }
}
