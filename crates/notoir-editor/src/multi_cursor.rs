//! Multi-cursor and selection commands (docs/specs/11 §Cursor and
//! Selection): add cursors above/below, select next/all occurrences, column
//! selection, expand/shrink selection and per-cursor paste.

use ropey::Rope;

use crate::display_map::line_text;
use crate::selection::{Range, Selection};
use crate::wrap::{char_at_col, col_of};

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word touching `pos` (empty when there is none).
pub fn word_at(rope: &Rope, pos: usize) -> Range {
    let len = rope.len_chars();
    let pos = pos.min(len);
    let mut start = pos;
    while start > 0 && is_word(rope.char(start - 1)) {
        start -= 1;
    }
    let mut end = pos;
    while end < len && is_word(rope.char(end)) {
        end += 1;
    }
    Range::new(start, end)
}

/// Adds a caret one line above the topmost (or below the bottommost) caret
/// at the same display column; it becomes the primary.
pub fn add_cursor_vertical(rope: &Rope, sel: &Selection, up: bool, tab: usize) -> Selection {
    let ranges = sel.ranges();
    let edge = if up {
        ranges.iter().min_by_key(|r| r.head)
    } else {
        ranges.iter().max_by_key(|r| r.head)
    };
    let Some(edge) = edge else {
        return sel.clone();
    };
    let line = rope.char_to_line(edge.head.min(rope.len_chars()));
    let target = if up {
        match line.checked_sub(1) {
            Some(l) => l,
            None => return sel.clone(),
        }
    } else if line + 1 < rope.len_lines() {
        line + 1
    } else {
        return sel.clone();
    };
    let col = col_of(
        &line_text(rope, line),
        edge.head - rope.line_to_char(line),
        tab,
    );
    let text = line_text(rope, target);
    let pos = rope.line_to_char(target) + char_at_col(&text, col, tab).min(text.chars().count());
    let mut v = ranges.to_vec();
    v.push(Range::caret(pos));
    let n = v.len() - 1;
    Selection::new(v, n)
}

fn find_from(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Select Next Occurrence (`Mod+D`): an empty primary selects the word at
/// the caret; otherwise the next match of the primary's text after the last
/// range (wrapping) is added as the new primary. `None` when nothing changes.
pub fn select_next_occurrence(rope: &Rope, sel: &Selection) -> Option<Selection> {
    let p = sel.primary();
    if p.is_empty() {
        let w = word_at(rope, p.head);
        if w.is_empty() {
            return None;
        }
        let mut v = sel.ranges().to_vec();
        v[sel.primary_index()] = w;
        return Some(Selection::new(v, sel.primary_index()));
    }
    let needle: Vec<char> = rope.slice(p.from()..p.to()).chars().collect();
    let hay: Vec<char> = rope.chars().collect();
    let last_end = sel.ranges().iter().map(Range::to).max().unwrap_or(0);
    let taken = |s: usize| sel.ranges().iter().any(|r| r.from() == s);
    let next = find_from(&hay, &needle, last_end)
        .filter(|s| !taken(*s))
        .or_else(|| {
            let mut i = 0;
            while let Some(s) = find_from(&hay, &needle, i) {
                if !taken(s) {
                    return Some(s);
                }
                i = s + 1;
            }
            None
        })?;
    let mut v = sel.ranges().to_vec();
    v.push(Range::new(next, next + needle.len()));
    let n = v.len() - 1;
    Some(Selection::new(v, n))
}

/// Select All Occurrences (`Mod+Shift+L`) of the primary's text, or of the
/// word at the caret.
pub fn select_all_occurrences(rope: &Rope, sel: &Selection) -> Option<Selection> {
    let mut p = sel.primary();
    if p.is_empty() {
        p = word_at(rope, p.head);
        if p.is_empty() {
            return None;
        }
    }
    let needle: Vec<char> = rope.slice(p.from()..p.to()).chars().collect();
    let hay: Vec<char> = rope.chars().collect();
    let mut v = Vec::new();
    let mut primary = 0;
    let mut i = 0;
    while let Some(s) = find_from(&hay, &needle, i) {
        if s == p.from() {
            primary = v.len();
        }
        v.push(Range::new(s, s + needle.len()));
        i = s + needle.len();
    }
    Some(Selection::new(v, primary))
}

/// Column (box) selection between two points given as (line, display
/// column); the head of each range is on the `head` column.
pub fn column_selection(
    rope: &Rope,
    anchor: (usize, usize),
    head: (usize, usize),
    tab: usize,
) -> Selection {
    let (l0, l1) = (anchor.0.min(head.0), anchor.0.max(head.0));
    let ranges: Vec<Range> = (l0..=l1.min(rope.len_lines().saturating_sub(1)))
        .map(|l| {
            let text = line_text(rope, l);
            let start = rope.line_to_char(l);
            let n = text.chars().count();
            let a = start + char_at_col(&text, anchor.1, tab).min(n);
            let h = start + char_at_col(&text, head.1, tab).min(n);
            Range::new(a, h)
        })
        .collect();
    let primary = if head.0 >= anchor.0 {
        ranges.len().saturating_sub(1)
    } else {
        0
    };
    Selection::new(ranges, primary)
}

/// Pastes one line per range when the text has exactly as many lines as
/// there are ranges; `None` otherwise (paste the whole text everywhere).
pub fn distribute_lines(sel: &Selection, text: &str) -> Option<Vec<String>> {
    let n = sel.len();
    if n < 2 {
        return None;
    }
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<String> = trimmed.split('\n').map(str::to_owned).collect();
    (lines.len() == n).then_some(lines)
}

const BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
const QUOTES: [char; 3] = ['"', '\'', '`'];
/// Bracket search stops this far from the selection.
const SCAN_LIMIT: usize = 100_000;

/// Innermost bracket pair around `from..to`: (open, close) positions.
fn enclosing_brackets(chars: &[char], from: usize, to: usize) -> Option<(usize, usize)> {
    let lo = from.saturating_sub(SCAN_LIMIT);
    let mut depth = [0usize; 3];
    let mut i = from;
    while i > lo {
        i -= 1;
        let c = chars[i];
        for (k, (o, cl)) in BRACKETS.iter().enumerate() {
            if c == *cl {
                depth[k] += 1;
            } else if c == *o {
                if depth[k] == 0 {
                    // Find the matching close after `to`.
                    let mut d = 0usize;
                    let hi = (to + SCAN_LIMIT).min(chars.len());
                    let mut j = i + 1;
                    while j < hi {
                        if chars[j] == *o {
                            d += 1;
                        } else if chars[j] == *cl {
                            if d == 0 {
                                return (j >= to).then_some((i, j));
                            }
                            d -= 1;
                        }
                        j += 1;
                    }
                    return None;
                }
                depth[k] -= 1;
            }
        }
    }
    None
}

/// Quote pair on the same line around `from..to`.
fn enclosing_quotes(rope: &Rope, from: usize, to: usize) -> Option<(usize, usize)> {
    let line = rope.char_to_line(from);
    if rope.char_to_line(to.min(rope.len_chars())) != line {
        return None;
    }
    let start = rope.line_to_char(line);
    let text: Vec<char> = line_text(rope, line).chars().collect();
    let (a, b) = (from - start, to - start);
    QUOTES
        .iter()
        .filter_map(|q| {
            let open = text[..a].iter().rposition(|c| c == q)?;
            // Only when the quote opens a string (even count before it).
            if text[..open].iter().filter(|c| *c == q).count() % 2 != 0 {
                return None;
            }
            let close = b + text[b..].iter().position(|c| c == q)?;
            Some((start + open, start + close))
        })
        .max_by_key(|p| p.0)
}

/// The next larger range strictly containing `r`: word, quote or bracket
/// contents, the pair itself, the line, the paragraph, the document.
pub fn expand_range(rope: &Rope, r: Range) -> Range {
    let (from, to) = (r.from(), r.to());
    let len = rope.len_chars();
    let contains = |c: &Range| c.from() <= from && to <= c.to() && c.to() - c.from() > to - from;
    let mut cands: Vec<Range> = Vec::new();
    if r.is_empty() {
        cands.push(word_at(rope, from));
    }
    let chars: Vec<char> = rope.chars().collect();
    if let Some((o, c)) = enclosing_brackets(&chars, from, to) {
        cands.push(Range::new(o + 1, c));
        cands.push(Range::new(o, c + 1));
    }
    if let Some((o, c)) = enclosing_quotes(rope, from, to) {
        cands.push(Range::new(o + 1, c));
        cands.push(Range::new(o, c + 1));
    }
    let (l0, l1) = (rope.char_to_line(from), rope.char_to_line(to.min(len)));
    let line_end = |l: usize| rope.line_to_char(l) + line_text(rope, l).chars().count();
    cands.push(Range::new(rope.line_to_char(l0), line_end(l1)));
    let blank = |l: usize| line_text(rope, l).trim().is_empty();
    let mut p0 = l0;
    while p0 > 0 && !blank(p0 - 1) {
        p0 -= 1;
    }
    let mut p1 = l1;
    while p1 + 1 < rope.len_lines() && !blank(p1 + 1) {
        p1 += 1;
    }
    cands.push(Range::new(rope.line_to_char(p0), line_end(p1)));
    cands.push(Range::new(0, len));
    cands
        .into_iter()
        .filter(contains)
        .min_by_key(|c| c.to() - c.from())
        .unwrap_or(r)
}

/// Expand Selection on every range.
pub fn expand_selection(rope: &Rope, sel: &Selection) -> Selection {
    let ranges = sel
        .ranges()
        .iter()
        .map(|r| expand_range(rope, *r))
        .collect();
    Selection::new(ranges, sel.primary_index())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_cursor_above_and_below_keep_column() {
        let rope = Rope::from_str("hello\nhi\nworld");
        let sel = Selection::caret(rope.line_to_char(1) + 2); // end of "hi"
        let below = add_cursor_vertical(&rope, &sel, false, 4);
        assert_eq!(
            below.ranges(),
            &[Range::caret(8), Range::caret(rope.line_to_char(2) + 2)]
        );
        assert_eq!(below.primary(), Range::caret(11));
        let above = add_cursor_vertical(&rope, &below, true, 4);
        assert_eq!(above.len(), 3);
        assert_eq!(above.primary(), Range::caret(2));
        // At the first line nothing changes.
        let top = add_cursor_vertical(&rope, &Selection::caret(1), true, 4);
        assert_eq!(top.len(), 1);
    }

    #[test]
    fn select_next_and_all_occurrences() {
        let rope = Rope::from_str("foo bar foo baz foo");
        let s = select_next_occurrence(&rope, &Selection::caret(1)).unwrap();
        assert_eq!(s.ranges(), &[Range::new(0, 3)]);
        let s = select_next_occurrence(&rope, &s).unwrap();
        assert_eq!(s.ranges(), &[Range::new(0, 3), Range::new(8, 11)]);
        assert_eq!(s.primary(), Range::new(8, 11));
        let s = select_next_occurrence(&rope, &s).unwrap();
        assert_eq!(s.len(), 3);
        // All taken: nothing new.
        let again = select_next_occurrence(&rope, &s);
        assert!(again.is_none());
        let all = select_all_occurrences(&rope, &Selection::caret(9)).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all.primary(), Range::new(8, 11));
        // Wraps around to earlier matches.
        let from_last = Selection::single(Range::new(16, 19));
        let s = select_next_occurrence(&rope, &from_last).unwrap();
        assert_eq!(s.ranges()[0], Range::new(0, 3));
    }

    #[test]
    fn column_selection_and_paste_distribution() {
        let rope = Rope::from_str("abcdef\nab\nabcdef");
        let s = column_selection(&rope, (0, 1), (2, 4), 4);
        assert_eq!(
            s.ranges(),
            &[Range::new(1, 4), Range::new(8, 9), Range::new(11, 14)]
        );
        assert_eq!(distribute_lines(&s, "x\ny\nz\n").unwrap(), ["x", "y", "z"]);
        assert!(distribute_lines(&s, "x\ny").is_none());
        assert!(distribute_lines(&Selection::caret(0), "x").is_none());
    }

    #[test]
    fn expand_grows_through_structure() {
        let rope = Rope::from_str("fn f(a, \"hi there\") {\n    x\n}\n\nnext");
        let at = 11; // just after "hi", inside the string
        let r = expand_range(&rope, Range::caret(at));
        assert_eq!(rope.slice(r.from()..r.to()).to_string(), "hi");
        let r = expand_range(&rope, r);
        assert_eq!(rope.slice(r.from()..r.to()).to_string(), "hi there");
        let r = expand_range(&rope, r);
        assert_eq!(rope.slice(r.from()..r.to()).to_string(), "\"hi there\"");
        let r = expand_range(&rope, r);
        assert_eq!(rope.slice(r.from()..r.to()).to_string(), "a, \"hi there\"");
        let r = expand_range(&rope, r);
        assert_eq!(
            rope.slice(r.from()..r.to()).to_string(),
            "(a, \"hi there\")"
        );
        let r = expand_range(&rope, r);
        assert_eq!(
            rope.slice(r.from()..r.to()).to_string(),
            "fn f(a, \"hi there\") {"
        );
        let r = expand_range(&rope, r);
        assert_eq!(
            rope.slice(r.from()..r.to()).to_string(),
            "fn f(a, \"hi there\") {\n    x\n}"
        );
        let r = expand_range(&rope, r);
        assert_eq!(r, Range::new(0, rope.len_chars()));
    }
}
