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
pub mod minimap;
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
#[derive(Clone, Debug, PartialEq)]
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
    /// Find matches to highlight (char ranges) and the current one.
    pub find_matches: std::sync::Arc<Vec<(usize, usize)>>,
    pub find_current: Option<usize>,
    /// Centre the caret this frame (Go to Line, find next).
    pub center: bool,
    /// Scroll to this position this frame (Go to Line cancel).
    pub set_scroll: Option<notoir_core::view::ScrollPosition>,
    /// `None` when hidden (setting, large file).
    pub minimap: Option<minimap::MinimapOptions>,
    /// Zen Mode: centre a column this many characters wide.
    pub max_width_cols: Option<usize>,
    /// Syntax highlighting (off in large-file mode).
    pub highlight: bool,
    /// Error ranges drawn with a squiggle (JSON syntax errors).
    pub diagnostics: Vec<(usize, usize)>,
    /// Bookmarked lines (spec 12).
    pub bookmarks: Vec<usize>,
    /// Foldable regions; `None` when folding is off.
    pub fold_ranges: Option<std::sync::Arc<Vec<notoir_editor::folding::FoldRange>>>,
    /// A pane of a diff tab: changed lines, intra-line changes and fillers.
    pub diff: Option<DiffDecor>,
    /// Git change bars against the index / HEAD (spec 28).
    pub git_changes: Option<std::sync::Arc<Vec<notoir_editor::diff::LineChange>>>,
    /// Scroll to these pixel offsets (vertical, horizontal) this frame
    /// (synchronized diff panes).
    pub set_scroll_px: Option<(f32, f32)>,
    /// Selection → Column Selection Mode: dragging selects a box.
    pub column_mode: bool,
}

/// Diff colouring of one side (docs/specs/27 §Diff View).
#[derive(Clone, Debug, PartialEq)]
pub struct DiffDecor {
    /// Right side: inserted colours and `+`; left: removed and `−`.
    pub inserted: bool,
    /// Changed line ranges, sorted.
    pub lines: std::sync::Arc<Vec<std::ops::Range<usize>>>,
    /// Intra-line changes: (line, start char, end char), sorted by line.
    pub spans: std::sync::Arc<Vec<(usize, usize, usize)>>,
    /// Filler rows aligning the sides: (before line, rows).
    pub fillers: std::sync::Arc<Vec<(usize, usize)>>,
}

impl DiffDecor {
    pub fn changed(&self, line: usize) -> bool {
        let i = self.lines.partition_point(|r| r.end <= line);
        self.lines.get(i).is_some_and(|r| r.start <= line)
    }
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
    /// Primary caret last frame: a change from outside the widget (commands,
    /// find) scrolls it into view.
    last_head: usize,
    minimap_rect: Option<Rect>,
    gutter_rect: Rect,
    /// `⋯` chips of folded regions drawn last frame (click unfolds).
    fold_chips: Vec<(Rect, usize)>,
    /// Line right-clicked in the gutter (its context menu acts on it).
    gutter_menu_line: Option<usize>,
    /// Syntax highlighting state for the language below (spec 26).
    highlighter: Option<notoir_editor::syntax::Highlighter>,
    highlight_lang: Option<notoir_editor::LanguageId>,
    /// Last frame's scroll offsets, row height and text top (diff panes).
    last_scroll: (f32, f32),
    last_row_h: f32,
    last_text_top: f32,
}

impl ViewCache {
    fn new(
        doc: &Document,
        wrap: Option<WrapConfig>,
        scroll: notoir_core::view::ScrollPosition,
        head: usize,
    ) -> Self {
        let rope = doc.buffer.rope().clone();
        Self {
            revision: doc.revision,
            map: DisplayMap::new(&rope, wrap),
            rope,
            desired_col: None,
            preedit: None,
            anchor_line: scroll
                .top_line
                .min(doc.buffer.rope().len_lines().saturating_sub(1)),
            anchor_px: scroll.pixel_offset,
            scroll_x: scroll.horizontal,
            blink_epoch: 0.0,
            drag: None,
            reveal: false,
            last_head: head,
            minimap_rect: None,
            gutter_rect: Rect::NOTHING,
            fold_chips: Vec::new(),
            gutter_menu_line: None,
            highlighter: None,
            highlight_lang: None,
            last_scroll: (0.0, 0.0),
            last_row_h: 0.0,
            last_text_top: 0.0,
        }
    }

    /// Line range an edit touched: (first line, old lines after it, new lines
    /// after it).
    fn changed_lines(
        before: &Rope,
        after: &Rope,
        cs: &notoir_editor::ChangeSet,
    ) -> (usize, usize, usize) {
        let edits = cs.edits();
        let (Some(first_edit), Some(last_edit)) = (edits.first(), edits.last()) else {
            return (0, 0, 0);
        };
        let first = before.char_to_line(first_edit.start.min(before.len_chars()));
        let old_last = before.char_to_line(last_edit.end.min(before.len_chars()));
        let new_end = cs
            .map_pos(last_edit.end, notoir_editor::Assoc::After)
            .min(after.len_chars());
        let new_last = after.char_to_line(new_end);
        (first, old_last - first, new_last.saturating_sub(first))
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
                        if let Some(h) = self.highlighter.as_mut() {
                            let (first, removed, inserted) =
                                Self::changed_lines(&before, &self.rope, &cs);
                            h.edited(first, removed, inserted);
                        }
                    }
                }
                None => {
                    self.rope = doc.buffer.rope().clone();
                    self.map = DisplayMap::new(&self.rope, wrap);
                    self.highlight_lang = None;
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

    /// Where the minimap and gutter were drawn last frame (tests).
    pub fn geometry(&self, view: ViewId) -> Option<(Option<Rect>, Rect)> {
        self.views
            .get(&view)
            .map(|c| (c.minimap_rect, c.gutter_rect))
    }

    /// Syntax spans of a line as of the last frame (tests).
    pub fn spans(&self, view: ViewId, line: usize) -> Option<Vec<notoir_editor::syntax::Span>> {
        let h = self.views.get(&view)?.highlighter.as_ref()?;
        h.spans(line).map(|s| s.to_vec())
    }

    /// First visible buffer line as of the last frame (tests).
    pub fn top_line(&self, view: ViewId) -> Option<usize> {
        self.views.get(&view).map(|c| c.anchor_line)
    }

    /// Scroll offsets in pixels (vertical, horizontal) as of the last frame.
    pub fn scroll_px(&self, view: ViewId) -> Option<(f32, f32)> {
        self.views.get(&view).map(|c| c.last_scroll)
    }

    /// Screen y of the first row of `line` (or of the fillers before it) as
    /// of the last frame, and the row height.
    pub fn line_y(&mut self, view: ViewId, line: usize, with_fillers: bool) -> Option<(f32, f32)> {
        let c = self.views.get_mut(&view)?;
        if c.last_row_h <= 0.0 {
            return None;
        }
        let mut row = c.map.line_to_row(line);
        if with_fillers {
            row -= c.map.fillers_before(line).min(row);
        }
        Some((
            c.last_text_top + row as f32 * c.last_row_h - c.last_scroll.0,
            c.last_row_h,
        ))
    }

    /// Display row of `line` (or of the fillers before it).
    pub fn line_row(&mut self, view: ViewId, line: usize, with_fillers: bool) -> Option<usize> {
        let c = self.views.get_mut(&view)?;
        let row = c.map.line_to_row(line);
        Some(if with_fillers {
            row - c.map.fillers_before(line).min(row)
        } else {
            row
        })
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
    /// Commands from the editor's own menus (minimap).
    pub commands: Vec<notoir_core::command::CommandInvocation>,
    /// The editor's response (the context menu is added by the caller).
    pub response: Option<egui::Response>,
    /// The context menu was opened on the gutter at this line.
    pub gutter_menu_line: Option<usize>,
}

/// Geometry of the editor for one frame.
#[derive(Clone)]
struct Metrics {
    rect: Rect,
    gutter: Rect,
    /// Bookmark lane (left edge of the gutter).
    bookmark_lane: Rect,
    /// Fold controls (right edge of the gutter), when folding is on.
    fold_lane: Option<Rect>,
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
/// Highlighting time per frame (spec 26: 2 ms; more in debug builds).
const HIGHLIGHT_BUDGET_MS: u64 = if cfg!(debug_assertions) { 8 } else { 3 };

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
    let lane_w = (char_w * 1.6).max(12.0);
    let fold_w = if opts.fold_ranges.is_some() {
        lane_w
    } else {
        0.0
    };
    let gutter_w = lane_w
        + fold_w
        + match opts.line_numbers {
            LineNumbers::Off => char_w * 0.5,
            _ => (gutter_digits(lines) as f32 + 1.0) * char_w,
        };
    // Zen Mode: a centred column.
    let rect = match opts.max_width_cols {
        Some(cols) => {
            let w = (cols as f32 * char_w + gutter_w + opts.padding * 2.0 + SCROLLBAR_W)
                .min(rect.width());
            Rect::from_center_size(rect.center(), Vec2::new(w, rect.height()))
        }
        None => rect,
    };
    // Minimap (hidden in narrow views).
    let mini = opts.minimap.filter(|_| rect.width() >= 400.0);
    let mini_rect = mini.map(|mm| {
        if mm.left {
            Rect::from_min_size(rect.min, Vec2::new(mm.width, rect.height()))
        } else {
            Rect::from_min_max(
                Pos2::new(rect.right() - SCROLLBAR_W - mm.width, rect.top()),
                Pos2::new(rect.right() - SCROLLBAR_W, rect.bottom()),
            )
        }
    });
    let left = match (mini, mini_rect) {
        (Some(mm), Some(r)) if mm.left => r.right(),
        _ => rect.left(),
    };
    let right = match (mini, mini_rect) {
        (Some(mm), Some(r)) if !mm.left => r.left(),
        _ => rect.right() - SCROLLBAR_W,
    };
    let gutter = Rect::from_min_size(
        Pos2::new(left, rect.top()),
        Vec2::new(gutter_w, rect.height()),
    );
    let text = Rect::from_min_max(
        Pos2::new(gutter.right() + opts.padding, rect.top()),
        Pos2::new(right, rect.bottom()),
    );
    let bookmark_lane = Rect::from_min_size(gutter.min, Vec2::new(lane_w, gutter.height()));
    let fold_lane = (fold_w > 0.0)
        .then(|| Rect::from_min_max(Pos2::new(gutter.right() - fold_w, gutter.top()), gutter.max));
    let m = Metrics {
        rect,
        gutter,
        bookmark_lane,
        fold_lane,
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

    let (saved_scroll, head) = wb
        .view(view)
        .map(|v| (v.scroll, v.selection.primary().head))
        .unwrap_or_default();
    let cache = caches
        .views
        .entry(view)
        .or_insert_with(|| ViewCache::new(doc, wrap, saved_scroll, head));
    cache.sync(doc, wrap);
    apply_folds(wb, cache, view);
    cache
        .map
        .set_fillers(opts.diff.as_ref().map_or(&[][..], |d| &d.fillers[..]));
    if let Some((y, x)) = opts.set_scroll_px {
        set_scroll_y(cache, &m, y);
        cache.scroll_x = x.max(0.0);
    }
    if let Some(s) = opts.set_scroll {
        let line = s.top_line.min(cache.rope.len_lines().saturating_sub(1));
        cache.anchor_line = line;
        cache.anchor_px = s.pixel_offset;
        cache.scroll_x = s.horizontal;
    }

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
        opts.column_mode,
        now,
        &mut out.commands,
    );
    if let Some(d) = wb
        .view(view)
        .map(|v| v.document_id)
        .and_then(|d| wb.document(d))
    {
        cache.sync(d, wrap);
    }
    apply_folds(wb, cache, view);
    cache
        .map
        .set_fillers(opts.diff.as_ref().map_or(&[][..], |d| &d.fillers[..]));
    scroll(ui, &response, cache, &m, opts, &mut out);
    let head = wb.view(view).map_or(0, |v| v.selection.primary().head);
    if head != cache.last_head {
        cache.reveal = true;
        cache.last_head = head;
    }
    if opts.center {
        let row = cache
            .map
            .row_of_char(&cache.rope, head.min(cache.rope.len_chars())) as f32;
        set_scroll_y(
            cache,
            &m,
            row * m.row_h - m.text.height() / 2.0 + m.row_h / 2.0,
        );
        cache.reveal = false;
    }
    if cache.reveal {
        reveal_caret(wb, cache, view, &m, opts.indent.tab_size, wrap.is_some());
        cache.reveal = false;
    }

    // Syntax highlighting up to the viewport bottom within a frame budget;
    // later frames continue (spec 26).
    let language = wb
        .view(view)
        .and_then(|v| wb.document(v.document_id))
        .map(|d| d.language);
    if !opts.highlight || language.is_none() {
        cache.highlighter = None;
        cache.highlight_lang = None;
    } else if cache.highlight_lang != language {
        cache.highlight_lang = language;
        cache.highlighter =
            language.and_then(|l| notoir_editor::syntax::Highlighter::new(l, &cache.rope));
    }
    if cache.highlighter.is_some() {
        let first_row = (scroll_y(cache, &m) / m.row_h).floor().max(0.0) as usize;
        let last_line = cache.map.row_to_line(first_row + m.visible_rows() + 1).line;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(HIGHLIGHT_BUDGET_MS);
        let rope = &cache.rope;
        if let Some(h) = cache.highlighter.as_mut()
            && !h.highlight_to(rope, last_line, deadline)
        {
            ui.ctx().request_repaint();
        }
    }
    let selection = wb
        .view(view)
        .map(|v| v.selection.clone())
        .unwrap_or_default();
    out.cjk = paint::paint(ui, kit, cache, &selection, &m, opts, focused, now);
    paint::scrollbar(ui, kit, id, cache, &m);
    cache.minimap_rect = mini_rect.filter(|_| mini.is_some());
    cache.gutter_rect = m.gutter;
    if let (Some(mm), Some(r)) = (mini, mini_rect) {
        minimap::show(
            ui,
            kit,
            id,
            r,
            cache,
            &m,
            opts,
            &mm,
            &selection,
            &mut out.commands,
        );
    }
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
    if response.secondary_clicked() {
        cache.gutter_menu_line = response
            .interact_pointer_pos()
            .filter(|p| m.gutter.contains(*p))
            .map(|p| line_at_y(cache, &m, p.y));
    }
    out.gutter_menu_line = cache.gutter_menu_line;
    out.response = Some(response.clone());
    cache.last_scroll = (scroll_y(cache, &m), cache.scroll_x);
    cache.last_row_h = m.row_h;
    cache.last_text_top = m.text.top();
    // Persist the scroll anchor in the view (session restore, M8).
    if let Some(v) = wb.views.get_mut(view) {
        v.scroll.top_line = cache.anchor_line;
        v.scroll.pixel_offset = cache.anchor_px;
        v.scroll.horizontal = cache.scroll_x;
    }
    out
}

/// Hides the view's folded lines; a caret that moved into a fold unfolds it
/// (spec 12: the cursor never rests inside a folded region).
fn apply_folds(wb: &mut Workbench, cache: &mut ViewCache, view: ViewId) {
    let Some(v) = wb.views.get_mut(view) else {
        return;
    };
    if !v.folds.is_empty() {
        let len = cache.rope.len_chars();
        let heads: Vec<usize> = v
            .selection
            .ranges()
            .iter()
            .map(|r| cache.rope.char_to_line(r.head.min(len)))
            .collect();
        for line in heads {
            v.unfold_line(&cache.rope, line);
        }
    }
    let hidden = v.hidden_lines(&cache.rope);
    cache.map.set_hidden(&hidden);
}

/// Buffer line at a screen y.
fn line_at_y(cache: &mut ViewCache, m: &Metrics, y: f32) -> usize {
    let row = ((y - m.text.top() + scroll_y(cache, m)) / m.row_h)
        .floor()
        .max(0.0) as usize;
    cache.map.row_to_line(row).line
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

    // spec 12: "Relative line numbers"
    #[test]
    fn relative_line_numbers() {
        let label = |line| paint::line_number_label(LineNumbers::Relative, line, 49);
        assert_eq!(label(46), 3, "line 47 shows 3");
        assert_eq!(label(49), 50, "line 50 shows 50");
        assert_eq!(paint::line_number_label(LineNumbers::On, 46, 49), 47);
    }

    // spec 12: "Line numbers width does not change while scrolling"
    #[test]
    fn gutter_width_depends_only_on_line_count() {
        assert_eq!(gutter_digits(9), 3);
        assert_eq!(gutter_digits(12_000), 5);
    }
}
