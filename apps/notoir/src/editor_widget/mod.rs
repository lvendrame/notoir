//! The text editor widget (docs/specs/11, 12 §Gutter, 14; AD-1).
//!
//! Renders only visible display rows from a [`DisplayMap`] kept in sync with
//! the document through its recent change sets. Every edit goes through
//! `Workbench::edit`, so undo, dirty state, other views and events behave the
//! same as for any other origin.

use std::collections::HashMap;

use egui::{Pos2, Rect, Sense, Vec2};
use notoir_core::document::Document;
use notoir_core::ids::ViewId;
use notoir_core::workbench::Workbench;
use notoir_editor::display_map::{DisplayMap, WrapConfig, line_text};
use notoir_editor::edit_ops::{EditResult, IndentStyle};
use notoir_editor::wrap::char_at_col;
use notoir_editor::{Rope, Selection};

use crate::shell::Kit;

mod keys;
mod paint;

/// Zoom steps in percent (spec 14).
pub const ZOOM_STEPS: &[u32] = &[50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

pub fn zoom_step(current: u32, up: bool) -> u32 {
    if up {
        ZOOM_STEPS
            .iter()
            .copied()
            .find(|z| *z > current)
            .unwrap_or(*ZOOM_STEPS.last().expect("steps"))
    } else {
        ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|z| *z < current)
            .unwrap_or(ZOOM_STEPS[0])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapMode {
    Off,
    On,
    Column(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineNumbers {
    Off,
    On,
    Relative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whitespace {
    None,
    Boundary,
    Selection,
    Trailing,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Line,
    Block,
    Underline,
}

/// Effective settings for one view this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub zoom: f32,
    pub indent: IndentStyle,
    pub wrap: WrapMode,
    pub line_numbers: LineNumbers,
    pub whitespace: Whitespace,
    pub highlight_line: bool,
    pub highlight_gutter: bool,
    pub cursor: CursorStyle,
    pub blink: bool,
    pub markdown_lists: bool,
    pub empty_selection_clipboard: bool,
    pub padding: f32,
}

/// Per-view state that only the widget needs.
pub struct ViewCache {
    revision: u64,
    rope: Rope,
    map: DisplayMap,
    /// Sticky column for vertical motion.
    desired_col: Option<usize>,
    preedit: Option<String>,
    /// Scroll anchor: buffer line at the top and pixels below its first row.
    anchor_line: usize,
    anchor_px: f32,
    scroll_x: f32,
    blink_epoch: f64,
    drag: Option<keys::Drag>,
    reveal: bool,
}

impl ViewCache {
    fn new(doc: &Document, wrap: Option<WrapConfig>) -> Self {
        let rope = doc.buffer.rope().clone();
        Self {
            revision: doc.revision,
            map: DisplayMap::new(&rope, wrap),
            rope,
            desired_col: None,
            preedit: None,
            anchor_line: 0,
            anchor_px: 0.0,
            scroll_x: 0.0,
            blink_epoch: 0.0,
            drag: None,
            reveal: false,
        }
    }

    /// Brings the snapshot and display map up to the document's revision.
    fn sync(&mut self, doc: &Document, wrap: Option<WrapConfig>) {
        if self.map.wrap() != wrap {
            self.rope = doc.buffer.rope().clone();
            self.map = DisplayMap::new(&self.rope, wrap);
        } else if self.revision != doc.revision {
            match doc.changes_since(self.revision) {
                Some(changes) => {
                    for cs in changes {
                        let before = self.rope.clone();
                        cs.apply(&mut self.rope);
                        self.map.apply(&before, &self.rope, &cs);
                    }
                }
                None => {
                    self.rope = doc.buffer.rope().clone();
                    self.map = DisplayMap::new(&self.rope, wrap);
                }
            }
        }
        debug_assert_eq!(self.rope.len_chars(), doc.buffer.len_chars());
        self.revision = doc.revision;
    }
}

#[derive(Default)]
pub struct EditorCaches {
    views: HashMap<ViewId, ViewCache>,
}

impl EditorCaches {
    /// Drops caches of closed views.
    pub fn retain(&mut self, alive: impl Fn(ViewId) -> bool) {
        self.views.retain(|v, _| alive(*v));
    }

    /// Display rows of a view as of its last frame (tests, diagnostics).
    pub fn total_rows(&mut self, view: ViewId) -> Option<usize> {
        self.views.get_mut(&view).map(|c| c.map.total_rows())
    }
}

/// What the app needs to know after the widget ran.
#[derive(Default)]
pub struct EditorOutput {
    pub focused: bool,
    /// Ctrl/Cmd + wheel: zoom one step in (+1) or out (-1).
    pub zoom_steps: i32,
    /// Clicked or dragged in: its view should become the active one.
    pub interacted: bool,
    /// CJK scripts in the visible text (fonts are loaded on demand).
    pub cjk: crate::fonts::CjkUse,
}

/// Geometry of the editor for one frame.
#[derive(Clone)]
struct Metrics {
    rect: Rect,
    gutter: Rect,
    text: Rect,
    row_h: f32,
    char_w: f32,
    font: egui::FontId,
}

impl Metrics {
    fn visible_rows(&self) -> usize {
        (self.text.height() / self.row_h).floor().max(1.0) as usize
    }
}

pub const SCROLLBAR_W: f32 = 10.0;

fn gutter_digits(lines: usize) -> usize {
    lines.max(1).to_string().len().max(3)
}

/// Shows the editor for `view` filling the available space.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    wb: &mut Workbench,
    caches: &mut EditorCaches,
    view: ViewId,
    opts: &EditorOptions,
    request_focus: bool,
    now: f64,
) -> EditorOutput {
    let mut out = EditorOutput::default();
    let Some(doc_id) = wb.view(view).map(|v| v.document_id) else {
        return out;
    };
    let rect = ui.available_rect_before_wrap();
    let id = egui::Id::new(("editor", view));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    ui.advance_cursor_after_rect(rect);
    if request_focus || response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let focused = response.has_focus();
    out.focused = focused;
    out.interacted = response.clicked() || response.drag_started() || response.double_clicked();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    // Metrics.
    let font_size = (opts.font_size * opts.zoom).max(4.0);
    let font = egui::FontId::monospace(font_size);
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
    let row_h = (font_size * opts.line_height).round().max(1.0);
    let doc = wb.document(doc_id).expect("view's document");
    let lines = doc.buffer.rope().len_lines();
    let gutter_w = match opts.line_numbers {
        LineNumbers::Off => char_w,
        _ => (gutter_digits(lines) as f32 + 2.0) * char_w,
    };
    let gutter = Rect::from_min_size(rect.min, Vec2::new(gutter_w, rect.height()));
    let text = Rect::from_min_max(
        Pos2::new(gutter.right() + opts.padding, rect.top()),
        Pos2::new(rect.right() - SCROLLBAR_W, rect.bottom()),
    );
    let m = Metrics {
        rect,
        gutter,
        text,
        row_h,
        char_w,
        font,
    };
    let max_cols = (text.width() / char_w).floor().max(10.0) as usize;
    let wrap = match opts.wrap {
        WrapMode::Off => None,
        WrapMode::On => Some(WrapConfig {
            cols: max_cols,
            tab: opts.indent.tab_size,
        }),
        WrapMode::Column(c) => Some(WrapConfig {
            cols: c.min(max_cols).max(10),
            tab: opts.indent.tab_size,
        }),
    };

    let cache = caches
        .views
        .entry(view)
        .or_insert_with(|| ViewCache::new(doc, wrap));
    cache.sync(doc, wrap);

    // Input.
    if focused {
        keys::handle(ui, wb, cache, view, opts, &m, now, &mut out);
    }
    keys::pointer(
        ui,
        &response,
        wb,
        cache,
        view,
        &m,
        opts.indent.tab_size,
        now,
    );
    if let Some(d) = wb
        .view(view)
        .map(|v| v.document_id)
        .and_then(|d| wb.document(d))
    {
        cache.sync(d, wrap);
    }
    scroll(ui, &response, cache, &m, opts, &mut out);
    if cache.reveal {
        reveal_caret(wb, cache, view, &m, opts.indent.tab_size, wrap.is_some());
        cache.reveal = false;
    }

    let selection = wb
        .view(view)
        .map(|v| v.selection.clone())
        .unwrap_or_default();
    out.cjk = paint::paint(ui, kit, cache, &selection, &m, opts, focused, now);
    paint::scrollbar(ui, kit, id, cache, &m);
    if focused {
        ime_output(ui, cache, &selection, &m, opts.indent.tab_size);
        let blink_phase = (now - cache.blink_epoch).rem_euclid(1.0);
        let wait = if blink_phase < 0.5 {
            0.5 - blink_phase
        } else {
            1.0 - blink_phase
        };
        if opts.blink {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.01)));
        }
    }
    accessibility(&response, cache, &m);
    // Persist the scroll anchor in the view (session restore, M8).
    if let Some(v) = wb.views.get_mut(view) {
        v.scroll.top_line = cache.anchor_line;
        v.scroll.pixel_offset = cache.anchor_px;
        v.scroll.horizontal = cache.scroll_x;
    }
    out
}

/// Scroll position in pixels from the anchor.
fn scroll_y(cache: &mut ViewCache, m: &Metrics) -> f32 {
    cache.map.line_to_row(cache.anchor_line) as f32 * m.row_h + cache.anchor_px
}

fn set_scroll_y(cache: &mut ViewCache, m: &Metrics, y: f32) {
    let total = cache.map.total_rows() as f32 * m.row_h;
    let max = (total - m.row_h).max(0.0);
    let y = y.clamp(0.0, max);
    let row = (y / m.row_h).floor() as usize;
    let r = cache.map.row_to_line(row);
    let line_row = cache.map.line_to_row(r.line);
    cache.anchor_line = r.line;
    cache.anchor_px = y - line_row as f32 * m.row_h;
}

fn scroll(
    ui: &egui::Ui,
    response: &egui::Response,
    cache: &mut ViewCache,
    m: &Metrics,
    opts: &EditorOptions,
    out: &mut EditorOutput,
) {
    if !response.hovered() {
        return;
    }
    // Ctrl/Cmd + wheel arrives as egui's zoom delta (UI zoom itself is off).
    let (delta, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
    if zoom > 1.0 {
        out.zoom_steps = 1;
        return;
    } else if zoom < 1.0 {
        out.zoom_steps = -1;
        return;
    }
    if delta.y != 0.0 {
        let y = scroll_y(cache, m) - delta.y;
        set_scroll_y(cache, m, y);
    }
    if delta.x != 0.0 && opts.wrap == WrapMode::Off {
        cache.scroll_x = (cache.scroll_x - delta.x).max(0.0);
    }
}

/// Keeps the primary caret inside the viewport (vertical and, without wrap,
/// horizontal).
fn reveal_caret(
    wb: &Workbench,
    cache: &mut ViewCache,
    view: ViewId,
    m: &Metrics,
    tab: usize,
    wrapped: bool,
) {
    let Some(head) = wb.view(view).map(|v| v.selection.primary().head) else {
        return;
    };
    let row = cache.map.row_of_char(&cache.rope, head) as f32;
    let y = scroll_y(cache, m);
    let top = row * m.row_h;
    if top < y {
        set_scroll_y(cache, m, top);
    } else if top + m.row_h > y + m.text.height() {
        set_scroll_y(cache, m, top + m.row_h - m.text.height());
    }
    if !wrapped {
        let line = cache.rope.char_to_line(head);
        let rel = head - cache.rope.line_to_char(line);
        let x =
            notoir_editor::wrap::col_of(&line_text(&cache.rope, line), rel, tab) as f32 * m.char_w;
        if x < cache.scroll_x {
            cache.scroll_x = (x - 4.0 * m.char_w).max(0.0);
        } else if x > cache.scroll_x + m.text.width() - m.char_w {
            cache.scroll_x = x - m.text.width() + 4.0 * m.char_w;
        }
    }
}

/// Char position under a screen point.
fn pos_at(cache: &mut ViewCache, m: &Metrics, p: Pos2, tab: usize) -> usize {
    let y = p.y - m.text.top() + scroll_y(cache, m);
    let row = (y / m.row_h).floor().max(0.0) as usize;
    if row >= cache.map.total_rows() {
        return cache.rope.len_chars();
    }
    let r = cache.map.row_to_line(row);
    let text = line_text(&cache.rope, r.line);
    let starts = cache.map.row_starts(&cache.rope, r.line);
    let start = starts[r.sub_row];
    let end = starts.get(r.sub_row + 1).copied();
    let row_text: String = match end {
        Some(e) => text.chars().skip(start).take(e - start).collect(),
        None => text.chars().skip(start).collect(),
    };
    let x = p.x - m.text.left() + cache.scroll_x;
    let col = (x / m.char_w).round().max(0.0) as usize;
    let mut rel = start + char_at_col(&row_text, col, tab);
    if let Some(e) = end
        && rel >= e
    {
        rel = e.saturating_sub(1).max(start);
    }
    cache.rope.line_to_char(r.line) + rel
}

fn caret_rect(cache: &mut ViewCache, m: &Metrics, pos: usize, tab: usize) -> Rect {
    let row = cache.map.row_of_char(&cache.rope, pos);
    let line = cache.rope.char_to_line(pos);
    let rel = pos - cache.rope.line_to_char(line);
    let starts = cache.map.row_starts(&cache.rope, line);
    let sub = starts.partition_point(|&s| s <= rel).saturating_sub(1);
    let text: String = line_text(&cache.rope, line)
        .chars()
        .skip(starts[sub])
        .collect();
    let col = notoir_editor::wrap::col_of(&text, rel - starts[sub], tab);
    let y = m.text.top() + row as f32 * m.row_h - scroll_y(cache, m);
    let x = m.text.left() + col as f32 * m.char_w - cache.scroll_x;
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, m.row_h))
}

fn ime_output(ui: &egui::Ui, cache: &mut ViewCache, sel: &Selection, m: &Metrics, tab: usize) {
    let cursor_rect = caret_rect(cache, m, sel.primary().head, tab);
    ui.ctx().output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            rect: m.text,
            cursor_rect,
            should_interrupt_composition: false,
        });
    });
}

/// Screen readers get the visible rows as the editor's text value.
fn accessibility(response: &egui::Response, cache: &mut ViewCache, m: &Metrics) {
    let first = (scroll_y(cache, m) / m.row_h).floor() as usize;
    let first_line = cache.map.row_to_line(first).line;
    let last_line = cache.map.row_to_line(first + m.visible_rows()).line;
    let visible: String = (first_line..=last_line)
        .map(|l| line_text(&cache.rope, l))
        .collect::<Vec<_>>()
        .join("\n");
    response.widget_info(|| egui::WidgetInfo::text_edit(true, "", &visible, ""));
}

/// Applies an edit result through the workbench (undo, dirty, other views).
fn apply_edit(wb: &mut Workbench, view: ViewId, res: EditResult, now: f64) -> bool {
    let now_ms = (now * 1000.0) as u64;
    match wb.edit(view, res.changes, res.selection, res.kind, now_ms) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(error = %e, "edit rejected");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_clamp_at_ends() {
        assert_eq!(zoom_step(100, true), 110);
        assert_eq!(zoom_step(100, false), 90);
        assert_eq!(zoom_step(300, true), 300);
        assert_eq!(zoom_step(50, false), 50);
        assert_eq!(zoom_step(105, false), 100);
    }

    // spec 12: "Line numbers width does not change while scrolling"
    #[test]
    fn gutter_width_depends_only_on_line_count() {
        assert_eq!(gutter_digits(9), 3);
        assert_eq!(gutter_digits(12_000), 5);
    }
}
