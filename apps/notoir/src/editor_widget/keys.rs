//! Keyboard, text, IME, clipboard and pointer input for the editor widget.
//! Shortcuts bound in the keymap were already consumed by the dispatcher;
//! what reaches here is typing, navigation and editing keys.

use egui::{Event, Key};
use notoir_core::ids::ViewId;
use notoir_core::workbench::Workbench;
use notoir_editor::edit_ops::{self, Unit};
use notoir_editor::motion;
use notoir_editor::{EditKind, Range, Selection};

use super::{
    EditorOptions, EditorOutput, Metrics, ViewCache, apply_edit, pos_at, scroll_y, set_scroll_y,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragMode {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug)]
pub struct Drag {
    /// Range selected by the initial click (word/line for double/triple).
    origin: Range,
    mode: DragMode,
}

fn selection(wb: &Workbench, view: ViewId) -> Selection {
    wb.view(view)
        .map(|v| v.selection.clone())
        .unwrap_or_default()
}

fn set_selection(wb: &mut Workbench, view: ViewId, sel: Selection) {
    let _ = wb.set_selection(view, sel);
}

/// Applies a caret motion to every range; `extend` keeps anchors. Without
/// `extend`, a horizontal motion collapses a non-empty range to the edge
/// in the motion's direction.
fn move_all(
    sel: &Selection,
    extend: bool,
    collapse_to: Option<bool>,
    f: impl Fn(usize) -> usize,
) -> Selection {
    let ranges = sel
        .ranges()
        .iter()
        .map(|r| {
            if !extend
                && !r.is_empty()
                && let Some(to_start) = collapse_to
            {
                return Range::caret(if to_start { r.from() } else { r.to() });
            }
            let head = f(r.head);
            if extend {
                Range::new(r.anchor, head)
            } else {
                Range::caret(head)
            }
        })
        .collect();
    Selection::new(ranges, sel.primary_index())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle(
    ui: &egui::Ui,
    wb: &mut Workbench,
    cache: &mut ViewCache,
    view: ViewId,
    opts: &EditorOptions,
    m: &Metrics,
    now: f64,
    _out: &mut EditorOutput,
) {
    let events = ui.input(|i| i.events.clone());
    let mac = cfg!(target_os = "macos");
    let tab = opts.indent.tab_size;
    let read_only = wb
        .view(view)
        .and_then(|v| wb.document(v.document_id))
        .is_some_and(|d| d.is_read_only());
    for event in events {
        let rope = cache.rope.clone();
        let sel = selection(wb, view);
        let mut edit: Option<edit_ops::EditResult> = None;
        let mut moved: Option<Selection> = None;
        let mut keep_column = false;
        match event {
            Event::Text(text) if !text.is_empty() && cache.preedit.is_none() => {
                let single = text.chars().count() == 1 && !text.contains('\n');
                edit = edit_ops::insert(
                    &sel,
                    &text,
                    if single {
                        EditKind::Insert
                    } else {
                        EditKind::Other
                    },
                );
            }
            Event::Paste(text) => {
                let text = notoir_editor::line_ending::normalize(&text);
                // Markdown: pasting a URL over a selection makes a link.
                let url = text.trim();
                let is_url = (url.starts_with("https://") || url.starts_with("http://"))
                    && !url.contains(char::is_whitespace);
                let p = sel.primary();
                let single_line = !p.is_empty()
                    && sel.ranges().len() == 1
                    && !rope.slice(p.from()..p.to()).chars().any(|c| c == '\n');
                edit = if opts.markdown_lists && is_url && single_line {
                    let label: String = rope.slice(p.from()..p.to()).to_string();
                    edit_ops::insert(&sel, &format!("[{label}]({url})"), EditKind::Other)
                } else {
                    edit_ops::insert(&sel, &text, EditKind::Other)
                };
            }
            Event::Copy | Event::Cut => {
                let source = if sel.is_empty() && opts.empty_selection_clipboard {
                    edit_ops::line_ranges(&rope, &sel)
                } else {
                    sel.clone()
                };
                let text = edit_ops::selected_text(&rope, &source);
                if !text.is_empty() {
                    ui.ctx().copy_text(text);
                    if matches!(event, Event::Cut) {
                        edit = edit_ops::insert(&source, "", EditKind::Other);
                    }
                }
            }
            Event::Ime(ime) => match ime {
                egui::ImeEvent::Preedit { text, .. } => {
                    cache.preedit = (!text.is_empty()).then_some(text);
                }
                egui::ImeEvent::Commit(text) => {
                    cache.preedit = None;
                    if !text.is_empty() {
                        edit = edit_ops::insert(&sel, &text, EditKind::Other);
                    }
                }
                _ => {}
            },
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if cache.preedit.is_none() => {
                let word = if mac { modifiers.alt } else { modifiers.ctrl };
                let line_mod = mac && modifiers.mac_cmd;
                let doc_mod = if mac {
                    modifiers.mac_cmd
                } else {
                    modifiers.ctrl
                };
                let extend = modifiers.shift;
                match key {
                    Key::ArrowLeft => {
                        moved = Some(if line_mod {
                            move_all(&sel, extend, None, |p| motion::line_home(&rope, p))
                        } else if word {
                            move_all(&sel, extend, None, |p| motion::prev_word(&rope, p))
                        } else {
                            move_all(&sel, extend, Some(true), |p| {
                                motion::prev_grapheme(&rope, p)
                            })
                        });
                    }
                    Key::ArrowRight => {
                        moved = Some(if line_mod {
                            move_all(&sel, extend, None, |p| motion::line_end(&rope, p))
                        } else if word {
                            move_all(&sel, extend, None, |p| motion::next_word(&rope, p))
                        } else {
                            move_all(&sel, extend, Some(false), |p| {
                                motion::next_grapheme(&rope, p)
                            })
                        });
                    }
                    Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                        let up = matches!(key, Key::ArrowUp | Key::PageUp);
                        if line_mod && matches!(key, Key::ArrowUp | Key::ArrowDown) {
                            let target = if up { 0 } else { rope.len_chars() };
                            moved = Some(move_all(&sel, extend, None, |_| target));
                        } else {
                            let rows = if matches!(key, Key::PageUp | Key::PageDown) {
                                m.visible_rows().saturating_sub(1).max(1)
                            } else {
                                1
                            };
                            let delta = if up { -(rows as isize) } else { rows as isize };
                            let col = *cache.desired_col.get_or_insert_with(|| {
                                motion::row_col(&rope, &mut cache.map, sel.primary().head, tab)
                            });
                            let map = &mut cache.map;
                            let ranges: Vec<Range> = sel
                                .ranges()
                                .iter()
                                .map(|r| {
                                    let head =
                                        motion::move_rows(&rope, map, r.head, delta, col, tab);
                                    if extend {
                                        Range::new(r.anchor, head)
                                    } else {
                                        Range::caret(head)
                                    }
                                })
                                .collect();
                            moved = Some(Selection::new(ranges, sel.primary_index()));
                            if rows > 1 {
                                let y = scroll_y(cache, m) + delta as f32 * m.row_h;
                                set_scroll_y(cache, m, y);
                            }
                            keep_column = true;
                        }
                    }
                    Key::Home => {
                        moved = Some(if doc_mod && !mac {
                            move_all(&sel, extend, None, |_| 0)
                        } else {
                            move_all(&sel, extend, None, |p| motion::line_home(&rope, p))
                        });
                    }
                    Key::End => {
                        let len = rope.len_chars();
                        moved = Some(if doc_mod && !mac {
                            move_all(&sel, extend, None, |_| len)
                        } else {
                            move_all(&sel, extend, None, |p| motion::line_end(&rope, p))
                        });
                    }
                    Key::Backspace if !read_only => {
                        let res = if line_mod {
                            let s = move_all(&sel, true, None, |p| motion::line_home(&rope, p));
                            edit_ops::insert(&s, "", EditKind::DeleteBackward)
                        } else {
                            edit_ops::delete_backward(
                                &rope,
                                &sel,
                                if word { Unit::Word } else { Unit::Grapheme },
                            )
                        };
                        edit = res;
                    }
                    Key::Delete if !read_only => {
                        edit = edit_ops::delete_forward(
                            &rope,
                            &sel,
                            if word { Unit::Word } else { Unit::Grapheme },
                        );
                    }
                    Key::Enter if !modifiers.command && !modifiers.alt => {
                        edit = edit_ops::newline(&rope, &sel, opts.markdown_lists);
                    }
                    Key::Tab if !modifiers.command && !modifiers.alt => {
                        edit = if modifiers.shift {
                            edit_ops::outdent(&rope, &sel, opts.indent)
                        } else {
                            edit_ops::indent(&rope, &sel, opts.indent)
                        };
                    }
                    Key::Escape if sel.len() > 1 => {
                        moved = Some(Selection::single(sel.primary()));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        if let Some(res) = edit
            && !read_only
            && apply_edit(wb, view, res, now)
        {
            sync_after(wb, cache, view);
            cache.desired_col = None;
            cache.reveal = true;
            cache.blink_epoch = now;
        }
        if let Some(s) = moved {
            set_selection(wb, view, s);
            if !keep_column {
                cache.desired_col = None;
            }
            cache.reveal = true;
            cache.blink_epoch = now;
        }
    }
}

fn sync_after(wb: &Workbench, cache: &mut ViewCache, view: ViewId) {
    if let Some(doc) = wb.view(view).and_then(|v| wb.document(v.document_id)) {
        let wrap = cache.map.wrap();
        cache.sync(doc, wrap);
    }
}

/// Word (or punctuation run) around `pos`.
fn word_at(rope: &notoir_editor::Rope, pos: usize) -> Range {
    let len = rope.len_chars();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let at = if pos < len {
        rope.char(pos)
    } else if pos > 0 {
        rope.char(pos - 1)
    } else {
        return Range::caret(0);
    };
    if at == '\n' {
        return Range::caret(pos);
    }
    let class = |c: char| (is_word(c), c.is_whitespace());
    let k = class(at);
    let mut start = pos.min(len);
    while start > 0 && rope.char(start - 1) != '\n' && class(rope.char(start - 1)) == k {
        start -= 1;
    }
    let mut end = pos.min(len);
    while end < len && rope.char(end) != '\n' && class(rope.char(end)) == k {
        end += 1;
    }
    Range::new(start, end)
}

fn line_at(rope: &notoir_editor::Rope, pos: usize) -> Range {
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let end = if line + 1 < rope.len_lines() {
        rope.line_to_char(line + 1)
    } else {
        rope.len_chars()
    };
    Range::new(start, end)
}

/// Click, Shift-click, drag, double-click (word), triple-click (line) and
/// gutter click (line).
#[allow(clippy::too_many_arguments)]
pub(super) fn pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    wb: &mut Workbench,
    cache: &mut ViewCache,
    view: ViewId,
    m: &Metrics,
    tab: usize,
    now: f64,
) {
    if !ui.input(|i| i.pointer.any_down()) {
        cache.drag = None;
    }
    let Some(pointer) = response.interact_pointer_pos() else {
        return;
    };
    let pos = pos_at(cache, m, pointer, tab);
    let rope = cache.rope.clone();
    let start = |mode: DragMode, r: Range, cache: &mut ViewCache, wb: &mut Workbench| {
        cache.drag = Some(Drag { origin: r, mode });
        set_selection(wb, view, Selection::single(r));
    };
    if response.triple_clicked() {
        start(DragMode::Line, line_at(&rope, pos), cache, wb);
    } else if response.double_clicked() {
        start(DragMode::Word, word_at(&rope, pos), cache, wb);
    } else if response.is_pointer_button_down_on() && cache.drag.is_none() {
        if pointer.x < m.text.left() {
            start(DragMode::Line, line_at(&rope, pos), cache, wb);
        } else if ui.input(|i| i.modifiers.shift) {
            let anchor = selection(wb, view).primary().anchor;
            cache.drag = Some(Drag {
                origin: Range::caret(anchor),
                mode: DragMode::Char,
            });
            set_selection(wb, view, Selection::single(Range::new(anchor, pos)));
        } else {
            start(DragMode::Char, Range::caret(pos), cache, wb);
        }
    } else if response.dragged()
        && let Some(drag) = cache.drag
    {
        let target = match drag.mode {
            DragMode::Char => Range::caret(pos),
            DragMode::Word => word_at(&rope, pos),
            DragMode::Line => line_at(&rope, pos),
        };
        let r = if target.from() < drag.origin.from() {
            Range::new(drag.origin.to(), target.from())
        } else {
            Range::new(drag.origin.from(), target.to())
        };
        set_selection(wb, view, Selection::single(r));
        // Auto-scroll near the edges while dragging.
        let edge = if pointer.y < m.text.top() + m.row_h {
            -m.row_h
        } else if pointer.y > m.text.bottom() - m.row_h {
            m.row_h
        } else {
            0.0
        };
        if edge != 0.0 {
            let y = scroll_y(cache, m) + edge;
            set_scroll_y(cache, m, y);
            ui.ctx().request_repaint();
        }
    } else {
        return;
    }
    cache.desired_col = None;
    cache.blink_epoch = now;
}
