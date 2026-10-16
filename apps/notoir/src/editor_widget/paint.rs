//! Painting: background, current line, selections, text, whitespace, carets,
//! IME preedit, gutter (line numbers) and the vertical scrollbar.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use notoir_editor::display_map::line_text;
use notoir_editor::wrap::{char_cols, col_of};
use notoir_editor::{Range, Selection};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    CursorStyle, EditorOptions, LineNumbers, Metrics, SCROLLBAR_W, ViewCache, Whitespace, scroll_y,
    set_scroll_y,
};
use crate::shell::Kit;
use crate::theme::Tok;

/// One visible display row.
struct Row {
    line: usize,
    sub_row: usize,
    /// Char offset of the row start in the document.
    start: usize,
    /// Chars in the row (excluding the line break).
    len: usize,
    text: String,
    /// Whether this row ends its line.
    last: bool,
    y: f32,
}

/// Visible text rows, and the y of visible filler rows (diff alignment).
fn visible_rows(cache: &mut ViewCache, m: &Metrics) -> (Vec<Row>, Vec<f32>) {
    let sy = scroll_y(cache, m);
    let first = (sy / m.row_h).floor().max(0.0) as usize;
    let total = cache.map.total_rows();
    let count = m.visible_rows() + 2;
    let mut rows = Vec::with_capacity(count);
    let mut current_line = usize::MAX;
    let mut line_chars: Vec<char> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut fillers = Vec::new();
    for row in first..(first + count).min(total) {
        if cache.map.is_filler_row(row) {
            fillers.push(m.text.top() + row as f32 * m.row_h - sy);
            continue;
        }
        let r = cache.map.row_to_line(row);
        if r.line != current_line {
            current_line = r.line;
            line_chars = line_text(&cache.rope, r.line).chars().collect();
            starts = cache.map.row_starts(&cache.rope, r.line);
        }
        let s = starts[r.sub_row.min(starts.len() - 1)];
        let e = starts
            .get(r.sub_row + 1)
            .copied()
            .unwrap_or(line_chars.len());
        rows.push(Row {
            line: r.line,
            sub_row: r.sub_row,
            start: cache.rope.line_to_char(r.line) + s,
            len: e - s,
            text: line_chars[s..e].iter().collect(),
            last: r.sub_row + 1 >= starts.len(),
            y: m.text.top() + row as f32 * m.row_h - sy,
        });
    }
    (rows, fillers)
}

/// Theme colour of a syntax token.
fn token_color(kit: &Kit, kind: notoir_editor::syntax::TokenKind, plain: Color32) -> Color32 {
    crate::theme::syntax_token(kind).map_or(plain, |t| kit.color(t))
}

/// Tabs expanded for text that starts at grid column `start`.
fn display_from(text: &str, start: usize, tab: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut col = start;
    for c in text.chars() {
        let w = char_cols(c, col, tab);
        if c == '\t' {
            out.extend(std::iter::repeat_n(' ', w));
        } else {
            out.push(c);
        }
        col += w;
    }
    out
}

/// Expands tabs to spaces so text columns line up with the grid.
fn display_text(text: &str, tab: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut col = 0;
    for c in text.chars() {
        let w = char_cols(c, col, tab);
        if c == '\t' {
            out.extend(std::iter::repeat_n(' ', w));
        } else {
            out.push(c);
        }
        col += w;
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &egui::Ui,
    kit: &mut Kit,
    cache: &mut ViewCache,
    sel: &Selection,
    m: &Metrics,
    opts: &EditorOptions,
    focused: bool,
    now: f64,
) -> crate::fonts::CjkUse {
    let mut cjk = crate::fonts::CjkUse::default();
    let painter = ui.painter_at(m.rect);
    let tab = opts.indent.tab_size;
    painter.rect_filled(m.rect, 0.0, kit.color(Tok::EditorBackground));
    painter.rect_filled(m.gutter, 0.0, kit.color(Tok::GutterBackground));
    let (rows, filler_ys) = visible_rows(cache, m);
    let primary = sel.primary();
    // Diff: filler rows (hatched), changed lines and intra-line changes.
    if let Some(d) = &opts.diff {
        let hatch = kit.color(Tok::Border);
        for &y in &filler_ys {
            let r = Rect::from_min_max(
                Pos2::new(m.gutter.left(), y),
                Pos2::new(m.rect.right(), y + m.row_h),
            );
            let fp = painter.with_clip_rect(r.intersect(m.rect));
            let mut x = r.left() - m.row_h;
            while x < r.right() {
                fp.line_segment(
                    [Pos2::new(x, r.bottom()), Pos2::new(x + m.row_h, r.top())],
                    Stroke::new(1.0, hatch),
                );
                x += 6.0;
            }
        }
        let (line_bg, text_bg) = if d.inserted {
            (Tok::DiffInsertedLine, Tok::DiffInsertedText)
        } else {
            (Tok::DiffRemovedLine, Tok::DiffRemovedText)
        };
        let x0 = m.text.left() - cache.scroll_x;
        for row in rows.iter().filter(|r| d.changed(r.line)) {
            let r = Rect::from_min_size(
                Pos2::new(m.gutter.left(), row.y),
                Vec2::new(m.rect.right() - m.gutter.left(), m.row_h),
            );
            painter.rect_filled(r, 0.0, kit.color(line_bg));
            let first = d.spans.partition_point(|s| s.0 < row.line);
            for &(_, a, b) in d.spans[first..].iter().take_while(|s| s.0 == row.line) {
                let off = row.start - cache.rope.line_to_char(row.line);
                let (a, b) = (a.max(off), b.min(off + row.len));
                if a >= b {
                    continue;
                }
                let ca = col_of(&row.text, a - off, tab);
                let cb = col_of(&row.text, b - off, tab);
                let rect = Rect::from_min_max(
                    Pos2::new(x0 + ca as f32 * m.char_w, row.y),
                    Pos2::new(x0 + cb as f32 * m.char_w, row.y + m.row_h),
                );
                painter
                    .with_clip_rect(m.text.intersect(m.rect))
                    .rect_filled(rect, 0.0, kit.color(text_bg));
            }
        }
    }
    let caret_line = cache
        .rope
        .char_to_line(primary.head.min(cache.rope.len_chars()));
    let text_painter = painter.with_clip_rect(m.text.intersect(m.rect));
    let x0 = m.text.left() - cache.scroll_x;
    let col_x = |col: usize| x0 + col as f32 * m.char_w;

    // Current line (every row of the caret's line) when nothing is selected.
    // Diff panes keep their change colours on the caret's line.
    let diff_line = opts.diff.as_ref().is_some_and(|d| d.changed(caret_line));
    if opts.highlight_line && sel.is_empty() && !diff_line {
        for row in rows.iter().filter(|r| r.line == caret_line) {
            let r = Rect::from_min_size(
                Pos2::new(m.gutter.right(), row.y),
                Vec2::new(m.rect.right() - m.gutter.right(), m.row_h),
            );
            painter.rect_filled(r, 0.0, kit.color(Tok::EditorLineHighlight));
        }
    }

    let sel_color = if focused {
        kit.color(Tok::EditorSelection)
    } else {
        kit.color(Tok::EditorSelectionInactive)
    };
    let text_color = kit.color(Tok::EditorForeground);
    let ws_color = kit.color(Tok::EditorWhitespace);
    let matches = &opts.find_matches;
    for row in &rows {
        let row_end = row.start + row.len;
        // Find matches (the current one stronger).
        if !matches.is_empty() {
            let first = matches.partition_point(|m| m.1 <= row.start);
            for (i, &(ma, mb)) in matches.iter().enumerate().skip(first) {
                if ma > row_end {
                    break;
                }
                let (a, b) = (ma.max(row.start), mb.min(row_end));
                if a >= b && !(a == b && mb > row_end) {
                    continue;
                }
                let ca = col_of(&row.text, a - row.start, tab);
                let cb = col_of(&row.text, b - row.start, tab);
                let color = if opts.find_current == Some(i) {
                    kit.color(Tok::EditorFindMatchCurrent)
                } else {
                    kit.color(Tok::EditorFindMatch)
                };
                let extra = if mb > row_end { m.char_w * 0.6 } else { 0.0 };
                let rect = Rect::from_min_max(
                    Pos2::new(col_x(ca), row.y),
                    Pos2::new(col_x(cb) + extra, row.y + m.row_h),
                );
                text_painter.rect_filled(rect, 2.0, color);
            }
        }
        // Wiki links: underlined; dashed in the warning colour when the
        // note does not exist (spec 34).
        for &(la, lb, missing) in opts.wiki_links.iter() {
            let (a, b) = (la.max(row.start), lb.min(row_end));
            if a >= b {
                continue;
            }
            let ca = col_of(&row.text, a - row.start, tab);
            let cb = col_of(&row.text, b - row.start, tab);
            let (x0, x1) = (col_x(ca), col_x(cb));
            let y = row.y + m.row_h - 2.0;
            if missing {
                let stroke = Stroke::new(1.0, kit.color(Tok::Warning));
                let mut x = x0;
                while x < x1 {
                    text_painter
                        .line_segment([Pos2::new(x, y), Pos2::new((x + 3.0).min(x1), y)], stroke);
                    x += 6.0;
                }
            } else {
                text_painter.line_segment(
                    [Pos2::new(x0, y), Pos2::new(x1, y)],
                    Stroke::new(1.0, kit.color(Tok::SyntaxLink)),
                );
            }
        }
        // MCP edits: a fading accent background (spec 29).
        for &(ea, eb, strength) in &opts.mcp_edits {
            let (a, b) = (ea.max(row.start), eb.min(row_end));
            if a >= b {
                continue;
            }
            let ca = col_of(&row.text, a - row.start, tab);
            let cb = col_of(&row.text, b - row.start, tab);
            let rect = Rect::from_min_max(
                Pos2::new(col_x(ca), row.y),
                Pos2::new(col_x(cb), row.y + m.row_h),
            );
            text_painter.rect_filled(
                rect,
                2.0,
                kit.color(Tok::Accent).gamma_multiply(0.35 * strength),
            );
        }
        // Diagnostics: a wavy underline.
        for &(da, db) in &opts.diagnostics {
            let (a, b) = (da.max(row.start), db.min(row_end + 1));
            if a >= b || a > row_end {
                continue;
            }
            let ca = col_of(&row.text, a - row.start, tab);
            let cb = col_of(&row.text, (b - row.start).min(row.len), tab).max(ca + 1);
            let (x0, x1) = (col_x(ca), col_x(cb));
            let y = row.y + m.row_h - 2.0;
            let mut pts = Vec::new();
            let mut x = x0;
            let mut up = true;
            while x <= x1 {
                pts.push(Pos2::new(x, if up { y - 1.5 } else { y + 1.0 }));
                x += 2.5;
                up = !up;
            }
            if pts.len() > 1 {
                text_painter.add(egui::Shape::line(
                    pts,
                    Stroke::new(1.2, kit.color(Tok::Danger)),
                ));
            }
        }
        // Selections.
        for r in sel.ranges().iter().filter(|r| !r.is_empty()) {
            let (a, b) = (r.from().max(row.start), r.to().min(row_end));
            let covers_break = row.last && r.to() > row_end && r.from() <= row_end;
            if a > b || (a == b && !covers_break) {
                continue;
            }
            let ca = col_of(&row.text, a - row.start, tab);
            let cb = col_of(&row.text, b - row.start, tab);
            let extra = if covers_break { m.char_w * 0.6 } else { 0.0 };
            let rect = Rect::from_min_max(
                Pos2::new(col_x(ca), row.y),
                Pos2::new(col_x(cb) + extra, row.y + m.row_h),
            );
            text_painter.rect_filled(rect, 2.0, sel_color);
        }
        // Text.
        cjk.scan(&row.text);
        let shown = display_text(&row.text, tab);
        let row_offset = row.start - cache.rope.line_to_char(row.line);
        let spans = cache
            .highlighter
            .as_ref()
            .and_then(|h| h.spans(row.line))
            .cloned();
        let color_at = |rel: usize| -> Color32 {
            let at = row_offset + rel;
            spans
                .as_ref()
                .and_then(|sp| sp.iter().find(|s| s.start <= at && at < s.end))
                .map_or(text_color, |s| token_color(kit, s.kind, text_color))
        };
        if let Some(sp) = spans.as_ref().filter(|_| shown.is_ascii()) {
            // Coloured runs on the column grid.
            let chars: Vec<char> = row.text.chars().collect();
            let mut bounds: Vec<usize> = vec![0, chars.len()];
            for s in sp.iter() {
                for b in [s.start, s.end] {
                    if b > row_offset && b < row_offset + chars.len() {
                        bounds.push(b - row_offset);
                    }
                }
            }
            bounds.sort_unstable();
            bounds.dedup();
            for w in bounds.windows(2) {
                let (a, b) = (w[0], w[1]);
                let piece: String = chars[a..b].iter().collect();
                let col = col_of(&row.text, a, tab);
                let text = display_from(&piece, col, tab);
                if text.trim().is_empty() {
                    continue;
                }
                text_painter.text(
                    Pos2::new(col_x(col), row.y + m.row_h / 2.0),
                    Align2::LEFT_CENTER,
                    text,
                    m.font.clone(),
                    color_at(a),
                );
            }
        } else if shown.is_ascii() {
            text_painter.text(
                Pos2::new(x0, row.y + m.row_h / 2.0),
                Align2::LEFT_CENTER,
                &shown,
                m.font.clone(),
                text_color,
            );
        } else {
            // Place each grapheme on its grid column so wide/fallback glyphs stay aligned.
            let mut col = 0;
            let mut char_idx = 0;
            for g in row.text.graphemes(true) {
                let first = g.chars().next().unwrap_or(' ');
                let w: usize = if first == '\t' {
                    char_cols('\t', col, tab)
                } else {
                    g.chars()
                        .map(|c| char_cols(c, col, tab))
                        .sum::<usize>()
                        .max(1)
                };
                if first != '\t' && first != ' ' {
                    text_painter.text(
                        Pos2::new(col_x(col), row.y + m.row_h / 2.0),
                        Align2::LEFT_CENTER,
                        g,
                        m.font.clone(),
                        color_at(char_idx),
                    );
                }
                col += w;
                char_idx += g.chars().count();
            }
        }
        // Whitespace markers.
        paint_whitespace(
            &text_painter,
            row,
            sel,
            opts.whitespace,
            tab,
            col_x,
            m,
            ws_color,
        );
    }

    // IME preedit, drawn at the caret with an underline.
    if let Some(pre) = &cache.preedit
        && let Some(rect) = caret_rect_in(&rows, primary.head, m, tab, &col_x)
    {
        let galley = text_painter.layout_no_wrap(pre.clone(), m.font.clone(), text_color);
        let bg = Rect::from_min_size(rect.min, Vec2::new(galley.size().x, m.row_h));
        text_painter.rect_filled(bg, 0.0, kit.color(Tok::EditorBackground));
        text_painter.galley(
            Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            text_color,
        );
        text_painter.line_segment(
            [
                Pos2::new(bg.left(), bg.bottom() - 2.0),
                Pos2::new(bg.right(), bg.bottom() - 2.0),
            ],
            Stroke::new(1.0, text_color),
        );
    }

    // Carets. The primary caret's place is kept for popups (`[[` completion).
    cache.last_caret = caret_rect_in(&rows, sel.primary().head, m, tab, &col_x);
    let blink_on = !opts.blink || (now - cache.blink_epoch).rem_euclid(1.0) < 0.5;
    if focused && blink_on && cache.preedit.is_none() {
        let color = kit.color(Tok::EditorCursor);
        for r in sel.ranges() {
            if let Some(rect) = caret_rect_in(&rows, r.head, m, tab, &col_x) {
                let caret = match opts.cursor {
                    CursorStyle::Line => Rect::from_min_size(rect.min, Vec2::new(2.0, m.row_h)),
                    CursorStyle::Block => {
                        Rect::from_min_size(rect.min, Vec2::new(m.char_w, m.row_h))
                    }
                    CursorStyle::Underline => Rect::from_min_size(
                        Pos2::new(rect.left(), rect.bottom() - 2.0),
                        Vec2::new(m.char_w, 2.0),
                    ),
                };
                text_painter.rect_filled(
                    caret,
                    0.0,
                    if opts.cursor == CursorStyle::Block {
                        color.gamma_multiply(0.6)
                    } else {
                        color
                    },
                );
            }
        }
    }

    // Folded regions: a `⋯` chip after the header line.
    cache.fold_chips.clear();
    for row in rows.iter().filter(|r| r.last) {
        if row.line + 1 < cache.rope.len_lines()
            && cache.map.is_hidden(row.line + 1)
            && !cache.map.is_hidden(row.line)
        {
            let x = col_x(col_of(&row.text, row.len, tab)) + m.char_w * 0.6;
            let chip = Rect::from_min_size(
                Pos2::new(x, row.y + m.row_h * 0.18),
                Vec2::new(m.char_w * 2.6, m.row_h * 0.64),
            );
            text_painter.rect_filled(chip, 3.0, kit.color(Tok::BgPanelRaised));
            text_painter.rect_stroke(
                chip,
                3.0,
                Stroke::new(1.0, kit.color(Tok::Border)),
                egui::StrokeKind::Inside,
            );
            // Three dots (no font has to carry U+22EF).
            let gap = chip.width() / 4.0;
            for i in 1..=3 {
                text_painter.circle_filled(
                    Pos2::new(chip.left() + gap * i as f32, chip.center().y),
                    1.3,
                    kit.color(Tok::TextSecondary),
                );
            }
            cache.fold_chips.push((chip, row.line));
        }
    }

    // Gutter lanes: bookmarks (a filled bookmark shape, not colour alone).
    let gp = painter.with_clip_rect(m.gutter);
    // Git change bars at the gutter's inner edge: added solid, modified
    // dashed, deleted a wedge between lines (shape, not colour alone).
    if let Some(changes) = &opts.git_changes {
        use notoir_editor::diff::LineChangeKind as K;
        let x1 = m.gutter.right();
        let x0 = x1 - 3.0;
        for row in &rows {
            let i = changes.partition_point(|c| c.end.max(c.start + 1) <= row.line);
            for c in changes[i..].iter().take_while(|c| c.start <= row.line) {
                match c.kind {
                    K::Added | K::Modified if row.line < c.end => {
                        let tok = if c.kind == K::Added {
                            Tok::GitAdded
                        } else {
                            Tok::GitModified
                        };
                        let r = Rect::from_min_max(
                            Pos2::new(x0, row.y),
                            Pos2::new(x1, row.y + m.row_h),
                        );
                        if c.kind == K::Added {
                            gp.rect_filled(r, 0.0, kit.color(tok));
                        } else {
                            let mut y = r.top();
                            while y < r.bottom() {
                                gp.rect_filled(
                                    Rect::from_min_max(
                                        Pos2::new(x0, y),
                                        Pos2::new(x1, (y + 3.0).min(r.bottom())),
                                    ),
                                    0.0,
                                    kit.color(tok),
                                );
                                y += 5.0;
                            }
                        }
                    }
                    K::Deleted if row.line == c.start && row.sub_row == 0 => {
                        let s = (m.row_h * 0.25).min(5.0);
                        gp.add(egui::Shape::convex_polygon(
                            vec![
                                Pos2::new(x0 - 2.0, row.y - s),
                                Pos2::new(x1, row.y),
                                Pos2::new(x0 - 2.0, row.y + s),
                            ],
                            kit.color(Tok::GitDeleted),
                            Stroke::NONE,
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    // Diff markers: `+` / `−` in the bookmark lane (not colour alone).
    if let Some(d) = &opts.diff {
        let (mark, tok) = if d.inserted {
            ("+", Tok::DiffGutterInserted)
        } else {
            ("\u{2212}", Tok::DiffGutterRemoved)
        };
        for row in rows.iter().filter(|r| r.sub_row == 0 && d.changed(r.line)) {
            gp.text(
                Pos2::new(m.bookmark_lane.center().x, row.y + m.row_h / 2.0),
                Align2::CENTER_CENTER,
                mark,
                m.font.clone(),
                kit.color(tok),
            );
        }
    }
    for row in rows.iter().filter(|r| r.sub_row == 0) {
        if opts.bookmarks.binary_search(&row.line).is_ok() {
            let c = Pos2::new(m.bookmark_lane.center().x, row.y + m.row_h / 2.0);
            let w = (m.bookmark_lane.width() * 0.5).min(8.0);
            let h = (m.row_h * 0.62).min(13.0);
            let top = c.y - h / 2.0;
            let pts = vec![
                Pos2::new(c.x - w / 2.0, top),
                Pos2::new(c.x + w / 2.0, top),
                Pos2::new(c.x + w / 2.0, top + h),
                Pos2::new(c.x, top + h * 0.72),
                Pos2::new(c.x - w / 2.0, top + h),
            ];
            gp.add(egui::Shape::convex_polygon(
                pts,
                kit.color(Tok::Accent),
                Stroke::NONE,
            ));
        }
    }
    // Fold controls: on gutter hover, always for folded regions.
    if let (Some(lane), Some(ranges)) = (m.fold_lane, opts.fold_ranges.as_ref()) {
        let hover = ui.rect_contains_pointer(m.gutter);
        for row in rows.iter().filter(|r| r.sub_row == 0) {
            let folded = row.line + 1 < cache.rope.len_lines() && cache.map.is_hidden(row.line + 1);
            let foldable = folded
                || ranges
                    .binary_search_by_key(&row.line, |r| r.start_line)
                    .is_ok();
            if !foldable || !(hover || folded) {
                continue;
            }
            let c = Pos2::new(lane.center().x, row.y + m.row_h / 2.0);
            let s = (m.row_h * 0.18).min(4.0);
            let color = kit.color(if folded {
                Tok::TextSecondary
            } else {
                Tok::TextTertiary
            });
            let pts = if folded {
                vec![
                    Pos2::new(c.x - s * 0.6, c.y - s),
                    Pos2::new(c.x + s * 0.6, c.y),
                    Pos2::new(c.x - s * 0.6, c.y + s),
                ]
            } else {
                vec![
                    Pos2::new(c.x - s, c.y - s * 0.6),
                    Pos2::new(c.x, c.y + s * 0.6),
                    Pos2::new(c.x + s, c.y - s * 0.6),
                ]
            };
            gp.add(egui::Shape::line(pts, Stroke::new(1.4, color)));
        }
    }

    // Line numbers.
    if opts.line_numbers != LineNumbers::Off {
        let numbers_right = m.fold_lane.map_or(m.gutter.right(), |r| r.left());
        let active = kit.color(Tok::GutterLineNumberActive);
        let normal = kit.color(Tok::GutterLineNumber);
        for row in rows.iter().filter(|r| r.sub_row == 0) {
            let is_current = row.line == caret_line;
            let n = line_number_label(opts.line_numbers, row.line, caret_line);
            let color = if is_current && opts.highlight_gutter {
                active
            } else {
                normal
            };
            gp.text(
                Pos2::new(numbers_right - m.char_w * 0.5, row.y + m.row_h / 2.0),
                Align2::RIGHT_CENTER,
                n.to_string(),
                m.font.clone(),
                color,
            );
        }
    }
    cjk
}

/// The number shown for `line` (0-based): relative distance to the caret's
/// line in relative mode (the caret's line shows its own number).
pub(super) fn line_number_label(mode: LineNumbers, line: usize, caret_line: usize) -> usize {
    match mode {
        LineNumbers::Relative if line != caret_line => line.abs_diff(caret_line),
        _ => line + 1,
    }
}

fn caret_rect_in(
    rows: &[Row],
    pos: usize,
    m: &Metrics,
    tab: usize,
    col_x: &impl Fn(usize) -> f32,
) -> Option<Rect> {
    // At a wrap boundary the caret belongs to the following row.
    let row = rows.iter().find(|r| {
        r.start <= pos && (pos < r.start + r.len || (r.last && pos == r.start + r.len))
    })?;
    let col = col_of(&row.text, pos - row.start, tab);
    Some(Rect::from_min_size(
        Pos2::new(col_x(col), row.y),
        Vec2::new(2.0, m.row_h),
    ))
}

#[allow(clippy::too_many_arguments)]
fn paint_whitespace(
    painter: &egui::Painter,
    row: &Row,
    sel: &Selection,
    mode: Whitespace,
    tab: usize,
    col_x: impl Fn(usize) -> f32,
    m: &Metrics,
    color: Color32,
) {
    if mode == Whitespace::None {
        return;
    }
    let chars: Vec<char> = row.text.chars().collect();
    let leading = chars.iter().take_while(|c| c.is_whitespace()).count();
    let trailing_from = chars.len() - chars.iter().rev().take_while(|c| c.is_whitespace()).count();
    let selected = |i: usize| {
        let p = row.start + i;
        sel.ranges()
            .iter()
            .any(|r: &Range| !r.is_empty() && p >= r.from() && p < r.to())
    };
    let mut col = 0;
    for (i, c) in chars.iter().enumerate() {
        let w = char_cols(*c, col, tab);
        let show = c.is_whitespace()
            && match mode {
                Whitespace::All => true,
                Whitespace::Selection => selected(i),
                Whitespace::Trailing => i >= trailing_from,
                Whitespace::Boundary => {
                    i < leading
                        || i >= trailing_from
                        || (*c == ' ' && chars.get(i + 1) == Some(&' '))
                }
                Whitespace::None => false,
            };
        if show {
            let center = Pos2::new(
                col_x(col) + m.char_w * w as f32 / 2.0,
                row.y + m.row_h / 2.0,
            );
            if *c == '\t' {
                let a = Pos2::new(col_x(col) + 2.0, center.y);
                let b = Pos2::new(col_x(col + w) - 3.0, center.y);
                painter.line_segment([a, b], Stroke::new(1.0, color));
                painter.line_segment([b, b + Vec2::new(-3.0, -3.0)], Stroke::new(1.0, color));
                painter.line_segment([b, b + Vec2::new(-3.0, 3.0)], Stroke::new(1.0, color));
            } else {
                painter.circle_filled(center, (m.char_w * 0.08).max(1.0), color);
            }
        }
        col += w;
    }
}

/// Thin vertical scrollbar; drag the thumb or click the track to jump.
pub(super) fn scrollbar(
    ui: &egui::Ui,
    kit: &mut Kit,
    id: egui::Id,
    cache: &mut ViewCache,
    m: &Metrics,
) {
    let track = Rect::from_min_max(
        Pos2::new(m.rect.right() - SCROLLBAR_W, m.rect.top()),
        m.rect.max,
    );
    let total = cache.map.total_rows() as f32 * m.row_h + m.text.height() - m.row_h;
    if total <= m.text.height() + 0.5 {
        return;
    }
    let sy = scroll_y(cache, m);
    let thumb_h = (m.text.height() / total * track.height()).max(24.0);
    let thumb_y =
        track.top() + sy / (total - m.text.height()).max(1.0) * (track.height() - thumb_h);
    let thumb = Rect::from_min_size(
        Pos2::new(track.left() + 2.0, thumb_y),
        Vec2::new(SCROLLBAR_W - 4.0, thumb_h),
    );
    let resp = ui.interact(track, id.with("vscroll"), Sense::click_and_drag());
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let frac = ((p.y - track.top() - thumb_h / 2.0) / (track.height() - thumb_h).max(1.0))
            .clamp(0.0, 1.0);
        set_scroll_y(cache, m, frac * (total - m.text.height()));
    }
    let color = if resp.hovered() || resp.dragged() {
        kit.color(Tok::MinimapViewportHover)
    } else {
        kit.color(Tok::MinimapViewport)
    };
    ui.painter().rect_filled(thumb, 3.0, color);
}
