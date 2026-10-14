//! Caret motions over the rope (docs/specs/11 §Cursor and Selection):
//! graphemes, words, smart Home/End, and display-row-aware vertical motion.

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

use crate::display_map::{DisplayMap, line_text};
use crate::wrap::{char_at_col, col_of};

/// Char offsets of grapheme boundaries inside `line` (including 0 and len).
fn grapheme_boundaries(line: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut chars = 0;
    for g in line.graphemes(true) {
        out.push(chars);
        chars += g.chars().count();
    }
    out.push(chars);
    out
}

/// Start of the grapheme before `pos` (crosses line breaks).
pub fn prev_grapheme(rope: &Rope, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    if pos == start {
        return pos - 1;
    }
    let rel = pos - start;
    let b = grapheme_boundaries(&line_text(rope, line));
    start + b.iter().rev().copied().find(|&x| x < rel).unwrap_or(0)
}

/// End of the grapheme at `pos` (crosses line breaks).
pub fn next_grapheme(rope: &Rope, pos: usize) -> usize {
    let len = rope.len_chars();
    if pos >= len {
        return len;
    }
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let text = line_text(rope, line);
    let rel = pos - start;
    if rel >= text.chars().count() {
        return pos + 1;
    }
    let b = grapheme_boundaries(&text);
    start + b.iter().copied().find(|&x| x > rel).unwrap_or(rel + 1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Ctrl/Alt+Left: start of the previous word. A line break is its own stop.
pub fn prev_word(rope: &Rope, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    if rope.char(pos - 1) == '\n' {
        return pos - 1;
    }
    let mut p = pos;
    while p > 0 && is_blank(rope.char(p - 1)) {
        p -= 1;
    }
    if p == 0 || rope.char(p - 1) == '\n' {
        return p;
    }
    let k = class(rope.char(p - 1));
    while p > 0 && class(rope.char(p - 1)) == k && rope.char(p - 1) != '\n' {
        p -= 1;
    }
    p
}

/// Ctrl/Alt+Right: end of the next word. A line break is its own stop.
pub fn next_word(rope: &Rope, pos: usize) -> usize {
    let len = rope.len_chars();
    if pos >= len {
        return len;
    }
    if rope.char(pos) == '\n' {
        return pos + 1;
    }
    let mut p = pos;
    while p < len && is_blank(rope.char(p)) {
        p += 1;
    }
    if p >= len || rope.char(p) == '\n' {
        return p;
    }
    let k = class(rope.char(p));
    while p < len && class(rope.char(p)) == k && rope.char(p) != '\n' {
        p += 1;
    }
    p
}

/// Smart Home: first non-whitespace char, or column 0 if already there.
pub fn line_home(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let indent = line_text(rope, line)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count();
    if pos == start + indent {
        start
    } else {
        start + indent
    }
}

pub fn line_end(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    rope.line_to_char(line) + line_text(rope, line).chars().count()
}

/// Visual column of `pos` within its display row (for sticky vertical motion).
pub fn row_col(rope: &Rope, map: &mut DisplayMap, pos: usize, tab: usize) -> usize {
    let line = rope.char_to_line(pos);
    let rel = pos - rope.line_to_char(line);
    let starts = map.row_starts(rope, line);
    let sub = starts.partition_point(|&s| s <= rel).saturating_sub(1);
    let text: String = line_text(rope, line).chars().skip(starts[sub]).collect();
    col_of(&text, rel - starts[sub], tab)
}

/// Moves `delta` display rows from `pos`, aiming for visual column `col`.
pub fn move_rows(
    rope: &Rope,
    map: &mut DisplayMap,
    pos: usize,
    delta: isize,
    col: usize,
    tab: usize,
) -> usize {
    let row = map.row_of_char(rope, pos) as isize;
    let total = map.total_rows() as isize;
    let target = row + delta;
    if target < 0 {
        return 0;
    }
    if target >= total {
        return rope.len_chars();
    }
    let r = map.row_to_line(target as usize);
    let text = line_text(rope, r.line);
    let starts = map.row_starts(rope, r.line);
    let row_start = starts[r.sub_row];
    let row_end = starts.get(r.sub_row + 1).copied();
    let row_text: String = match row_end {
        Some(end) => text.chars().skip(row_start).take(end - row_start).collect(),
        None => text.chars().skip(row_start).collect(),
    };
    let mut rel = row_start + char_at_col(&row_text, col, tab);
    // On a wrapped row, the row's end is the next row's start: stay before it.
    if let Some(end) = row_end
        && rel >= end
    {
        rel = end.saturating_sub(1).max(row_start);
    }
    rope.line_to_char(r.line) + rel
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_map::WrapConfig;

    #[test]
    fn graphemes_move_as_units() {
        let rope = Rope::from_str("a👍🏽b\ne\u{301}");
        assert_eq!(next_grapheme(&rope, 1), 3);
        assert_eq!(prev_grapheme(&rope, 3), 1);
        assert_eq!(next_grapheme(&rope, 4), 5, "over the newline");
        assert_eq!(prev_grapheme(&rope, 5), 4);
        assert_eq!(next_grapheme(&rope, 5), 7, "e + combining accent");
    }

    #[test]
    fn word_motions() {
        let lines = Rope::from_str("ab\n  cd");
        assert_eq!(next_word(&lines, 2), 3, "line break is a stop");
        assert_eq!(next_word(&lines, 3), 7);
        assert_eq!(prev_word(&lines, 5), 3);
        assert_eq!(prev_word(&lines, 3), 2);
        let rope = Rope::from_str("foo_bar, baz(qux)");
        assert_eq!(next_word(&rope, 0), 7);
        assert_eq!(next_word(&rope, 7), 8);
        assert_eq!(next_word(&rope, 8), 12);
        assert_eq!(prev_word(&rope, 12), 9);
        assert_eq!(prev_word(&rope, 9), 7);
    }

    #[test]
    fn smart_home_toggles() {
        let rope = Rope::from_str("    code");
        assert_eq!(line_home(&rope, 6), 4);
        assert_eq!(line_home(&rope, 4), 0);
        assert_eq!(line_end(&rope, 0), 8);
    }

    #[test]
    fn vertical_motion_keeps_column_and_respects_wrap() {
        let rope = Rope::from_str("abcdef\nxy\nabcdef");
        let mut m = DisplayMap::new(&rope, None);
        assert_eq!(
            move_rows(&rope, &mut m, 5, 1, 5, 4),
            9,
            "clamped to short line end"
        );
        assert_eq!(
            move_rows(&rope, &mut m, 9, 1, 5, 4),
            15,
            "sticky column restored"
        );

        let rope = Rope::from_str("hello world foo\nz");
        let mut m = DisplayMap::new(&rope, Some(WrapConfig { cols: 6, tab: 4 }));
        // Row 0 "hello ", row 1 "world ", row 2 "foo".
        assert_eq!(move_rows(&rope, &mut m, 1, 1, 1, 4), 7);
        assert_eq!(
            move_rows(&rope, &mut m, 0, 1, 99, 4),
            11,
            "stays on the wrapped row"
        );
        assert_eq!(move_rows(&rope, &mut m, 0, -1, 0, 4), 0);
        assert_eq!(move_rows(&rope, &mut m, 0, 99, 0, 4), rope.len_chars());
    }
}
