//! Line operations (docs/specs/11 §Commands): duplicate, delete, move, sort,
//! trim trailing whitespace, change case, insert line, join. Each returns one
//! change set (a single undo step) and the selection after it.

use ropey::Rope;

use crate::buffer::EditKind;
use crate::change::{ChangeSet, Edit};
use crate::display_map::line_text;
use crate::edit_ops::EditResult;
use crate::natural::natural_cmp;
use crate::selection::{Range, Selection};

/// Lines touched by the selection as merged (first, last) blocks. A range
/// ending at the start of a line does not include that line.
pub fn line_blocks(rope: &Rope, sel: &Selection) -> Vec<(usize, usize)> {
    let mut blocks: Vec<(usize, usize)> = sel
        .ranges()
        .iter()
        .map(|r| {
            let first = rope.char_to_line(r.from());
            let mut last = rope.char_to_line(r.to());
            if r.to() > r.from() && last > first && rope.line_to_char(last) == r.to() {
                last -= 1;
            }
            (first, last)
        })
        .collect();
    blocks.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for b in blocks {
        match merged.last_mut() {
            Some(m) if b.0 <= m.1 + 1 => m.1 = m.1.max(b.1),
            _ => merged.push(b),
        }
    }
    merged
}

fn lines(rope: &Rope, first: usize, last: usize) -> Vec<String> {
    (first..=last).map(|l| line_text(rope, l)).collect()
}

/// End of `line`'s text (before its line break).
fn line_content_end(rope: &Rope, line: usize) -> usize {
    rope.line_to_char(line) + line_text(rope, line).chars().count()
}

fn shift(sel: &Selection, f: impl Fn(&Range) -> isize) -> Selection {
    let ranges = sel
        .ranges()
        .iter()
        .map(|r| {
            let d = f(r);
            Range::new(
                (r.anchor as isize + d).max(0) as usize,
                (r.head as isize + d).max(0) as usize,
            )
        })
        .collect();
    Selection::new(ranges, sel.primary_index())
}

fn result(edits: Vec<Edit>, selection: Selection) -> Option<EditResult> {
    if edits.is_empty() {
        return None;
    }
    Some(EditResult {
        changes: ChangeSet::new(edits).ok()?,
        selection,
        kind: EditKind::Other,
    })
}

/// Duplicate Line: copies each block below itself; carets move to the copy.
pub fn duplicate_lines(rope: &Rope, sel: &Selection) -> Option<EditResult> {
    let blocks = line_blocks(rope, sel);
    let mut edits = Vec::new();
    for &(a, b) in &blocks {
        let text = lines(rope, a, b).join("\n");
        let end = line_content_end(rope, b);
        edits.push(Edit::new(end, end, format!("\n{text}")));
    }
    let selection = shift(sel, |r| {
        let line = rope.char_to_line(r.from());
        let mut d = 0isize;
        for &(a, b) in &blocks {
            let len = lines(rope, a, b).join("\n").chars().count() as isize + 1;
            // Blocks above shift the range down; its own block's copy goes
            // below the original, and the caret moves onto the copy.
            if a <= line {
                d += len;
            }
        }
        d
    });
    result(edits, selection)
}

/// Delete Line: removes each block with its line break.
pub fn delete_lines(rope: &Rope, sel: &Selection) -> Option<EditResult> {
    let blocks = line_blocks(rope, sel);
    let total = rope.len_lines();
    let mut edits = Vec::new();
    for &(a, b) in &blocks {
        let (start, end) = if b + 1 < total {
            (rope.line_to_char(a), rope.line_to_char(b + 1))
        } else if a > 0 {
            (line_content_end(rope, a - 1), rope.len_chars())
        } else {
            (0, rope.len_chars())
        };
        edits.push(Edit::new(start, end, ""));
    }
    let changes = ChangeSet::new(edits.clone()).ok()?;
    let carets: Vec<Range> = blocks
        .iter()
        .map(|&(a, _)| {
            let start = rope.line_to_char(a);
            Range::caret(changes.map_pos(start, crate::change::Assoc::Before))
        })
        .collect();
    let primary = sel.primary_index().min(carets.len() - 1);
    Some(EditResult {
        changes,
        selection: Selection::new(carets, primary),
        kind: EditKind::Other,
    })
}

/// Move Line Up/Down: swaps each block with the line next to it; the
/// selection moves with the text.
pub fn move_lines(rope: &Rope, sel: &Selection, down: bool) -> Option<EditResult> {
    let blocks = line_blocks(rope, sel);
    let total = rope.len_lines();
    let mut edits = Vec::new();
    let mut moves: Vec<((usize, usize), isize)> = Vec::new();
    for &(a, b) in &blocks {
        let (lo, hi, other) = if down {
            if b + 1 >= total {
                continue;
            }
            (a, b + 1, b + 1)
        } else {
            if a == 0 {
                continue;
            }
            (a - 1, b, a - 1)
        };
        let block = lines(rope, a, b);
        let other_text = line_text(rope, other);
        let new: Vec<String> = if down {
            std::iter::once(other_text.clone())
                .chain(block.iter().cloned())
                .collect()
        } else {
            block
                .iter()
                .cloned()
                .chain(std::iter::once(other_text.clone()))
                .collect()
        };
        let start = rope.line_to_char(lo);
        let end = line_content_end(rope, hi);
        edits.push(Edit::new(start, end, new.join("\n")));
        let d = other_text.chars().count() as isize + 1;
        moves.push(((a, b), if down { d } else { -d }));
    }
    let selection = shift(sel, |r| {
        let line = rope.char_to_line(r.from());
        moves
            .iter()
            .find(|((a, b), _)| *a <= line && line <= *b)
            .map_or(0, |(_, d)| *d)
    });
    result(edits, selection)
}

/// Sort Lines: the selected lines, or the whole document when nothing is
/// selected (natural, case-insensitive).
pub fn sort_lines(rope: &Rope, sel: &Selection, descending: bool) -> Option<EditResult> {
    let blocks = if sel.is_empty() {
        vec![(0, rope.len_lines().saturating_sub(1))]
    } else {
        line_blocks(rope, sel)
    };
    let mut edits = Vec::new();
    for &(a, b) in &blocks {
        let mut ls = lines(rope, a, b);
        // A trailing empty last line stays last.
        let keep_empty_end =
            b + 1 == rope.len_lines() && ls.last().is_some_and(String::is_empty) && ls.len() > 1;
        let tail = if keep_empty_end { ls.pop() } else { None };
        let before = ls.clone();
        ls.sort_by(|x, y| natural_cmp(x, y));
        if descending {
            ls.reverse();
        }
        if ls == before {
            continue;
        }
        ls.extend(tail);
        edits.push(Edit::new(
            rope.line_to_char(a),
            line_content_end(rope, b),
            ls.join("\n"),
        ));
    }
    result(edits, sel.clone())
}

/// Trim Trailing Whitespace in the selected lines (whole document without a
/// selection).
pub fn trim_trailing_whitespace(rope: &Rope, sel: &Selection) -> Option<EditResult> {
    let blocks = if sel.is_empty() {
        vec![(0, rope.len_lines().saturating_sub(1))]
    } else {
        line_blocks(rope, sel)
    };
    let mut edits = Vec::new();
    for &(a, b) in &blocks {
        for l in a..=b {
            let text = line_text(rope, l);
            let kept = text.trim_end_matches([' ', '\t']).chars().count();
            let total = text.chars().count();
            if kept < total {
                let start = rope.line_to_char(l);
                edits.push(Edit::new(start + kept, start + total, ""));
            }
        }
    }
    let changes = ChangeSet::new(edits.clone()).ok()?;
    if edits.is_empty() {
        return None;
    }
    let selection = sel.map(&changes);
    Some(EditResult {
        changes,
        selection,
        kind: EditKind::Other,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Upper,
    Lower,
    Title,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// The word around `pos` (empty when there is none).
pub fn word_at(rope: &Rope, pos: usize) -> (usize, usize) {
    let len = rope.len_chars();
    let mut a = pos.min(len);
    while a > 0 && is_word_char(rope.char(a - 1)) {
        a -= 1;
    }
    let mut b = pos.min(len);
    while b < len && is_word_char(rope.char(b)) {
        b += 1;
    }
    (a, b)
}

fn convert(text: &str, case: Case) -> String {
    match case {
        Case::Upper => text.to_uppercase(),
        Case::Lower => text.to_lowercase(),
        Case::Title => {
            let mut out = String::with_capacity(text.len());
            let mut start = true;
            for c in text.chars() {
                if is_word_char(c) {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = true;
                }
            }
            out
        }
    }
}

/// Change Case on each selection (or the word at each caret); selections are
/// kept around the converted text.
pub fn transform_case(rope: &Rope, sel: &Selection, case: Case) -> Option<EditResult> {
    let mut edits = Vec::new();
    let mut ranges = Vec::new();
    for r in sel.ranges() {
        let (a, b) = if r.is_empty() {
            word_at(rope, r.head)
        } else {
            (r.from(), r.to())
        };
        if a == b {
            ranges.push(*r);
            continue;
        }
        let text = rope.slice(a..b).to_string();
        let new = convert(&text, case);
        let n = new.chars().count();
        if new != text {
            edits.push(Edit::new(a, b, new.clone()));
        }
        // Upper-casing can change lengths (ß → SS); keep the range over it.
        ranges.push(if r.is_empty() {
            *r
        } else if r.anchor <= r.head {
            Range::new(a, a + n)
        } else {
            Range::new(a + n, a)
        });
    }
    if edits.is_empty() {
        return None;
    }
    let changes = ChangeSet::new(edits).ok()?;
    let mapped: Vec<Range> = ranges
        .iter()
        .map(|r| {
            Range::new(
                changes.map_pos(r.anchor, crate::change::Assoc::Before),
                changes.map_pos(r.head, crate::change::Assoc::Before),
            )
        })
        .collect();
    Some(EditResult {
        changes,
        selection: Selection::new(mapped, sel.primary_index()),
        kind: EditKind::Other,
    })
}

/// Insert Line Below/Above: a new line with the current line's indentation;
/// the caret goes there.
pub fn insert_line(rope: &Rope, sel: &Selection, below: bool) -> Option<EditResult> {
    let mut edits = Vec::new();
    let mut lines_done = Vec::new();
    for r in sel.ranges() {
        let line = rope.char_to_line(r.head);
        if lines_done.contains(&line) {
            continue;
        }
        lines_done.push(line);
        let text = line_text(rope, line);
        let indent: String = text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if below {
            let end = line_content_end(rope, line);
            edits.push(Edit::new(end, end, format!("\n{indent}")));
        } else {
            let start = rope.line_to_char(line);
            edits.push(Edit::new(start, start, format!("{indent}\n")));
        }
    }
    let changes = ChangeSet::new(edits.clone()).ok()?;
    let mut sorted = edits;
    sorted.sort_by_key(|e| e.start);
    let mut delta = 0isize;
    let carets: Vec<Range> = sorted
        .iter()
        .map(|e| {
            let start = (e.start as isize + delta) as usize;
            delta += e.inserted_len() as isize;
            let indent = e.inserted_len() - 1;
            Range::caret(if below {
                start + 1 + indent
            } else {
                start + indent
            })
        })
        .collect();
    Some(EditResult {
        changes,
        selection: Selection::new(carets, 0),
        kind: EditKind::Other,
    })
}

/// Join Lines: each block (or the caret's line and the next) becomes one
/// line, separated by single spaces.
pub fn join_lines(rope: &Rope, sel: &Selection) -> Option<EditResult> {
    let total = rope.len_lines();
    let mut edits = Vec::new();
    for (a, b) in line_blocks(rope, sel) {
        let b = if a == b { (b + 1).min(total - 1) } else { b };
        for l in a..b {
            let end = line_content_end(rope, l);
            let next = line_text(rope, l + 1);
            let lead = next.chars().take_while(|c| c.is_whitespace()).count();
            let trail = line_text(rope, l)
                .chars()
                .rev()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            let sep = if next.trim().is_empty() { "" } else { " " };
            edits.push(Edit::new(end - trail, rope.line_to_char(l + 1) + lead, sep));
        }
    }
    let changes = ChangeSet::new(edits.clone()).ok()?;
    if edits.is_empty() {
        return None;
    }
    let selection = sel.map(&changes);
    Some(EditResult {
        changes,
        selection,
        kind: EditKind::Other,
    })
}

/// Convert Indentation: rewrites leading whitespace of the selected lines
/// (the whole document without a selection) as spaces or tabs.
pub fn convert_indentation(
    rope: &Rope,
    sel: &Selection,
    style: crate::edit_ops::IndentStyle,
) -> Option<EditResult> {
    let blocks = if sel.is_empty() {
        vec![(0, rope.len_lines().saturating_sub(1))]
    } else {
        line_blocks(rope, sel)
    };
    let tab = style.tab_size.max(1);
    let mut edits = Vec::new();
    for &(a, b) in &blocks {
        for l in a..=b {
            let text = line_text(rope, l);
            let lead: String = text
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            if lead.is_empty() {
                continue;
            }
            let mut cols = 0;
            for c in lead.chars() {
                cols = if c == '\t' {
                    (cols / tab + 1) * tab
                } else {
                    cols + 1
                };
            }
            let new = if style.insert_spaces {
                " ".repeat(cols)
            } else {
                format!("{}{}", "\t".repeat(cols / tab), " ".repeat(cols % tab))
            };
            if new != lead {
                let start = rope.line_to_char(l);
                edits.push(Edit::new(start, start + lead.chars().count(), new));
            }
        }
    }
    if edits.is_empty() {
        return None;
    }
    let changes = ChangeSet::new(edits).ok()?;
    let selection = sel.map(&changes);
    Some(EditResult {
        changes,
        selection,
        kind: EditKind::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(rope: &Rope, r: &EditResult) -> String {
        let mut out = rope.clone();
        r.changes.apply(&mut out);
        out.to_string()
    }

    // spec 11: "Move line down"
    #[test]
    fn move_line_down_keeps_cursor_on_moved_line() {
        let rope = Rope::from_str("1\n2\n3");
        let r = move_lines(&rope, &Selection::caret(0), true).unwrap();
        let out = apply(&rope, &r);
        assert_eq!(out, "2\n1\n3");
        let after = Rope::from_str(&out);
        assert_eq!(after.char_to_line(r.selection.primary().head), 1);
        // Up again, and nothing at the edges.
        let back = move_lines(&after, &r.selection, false).unwrap();
        assert_eq!(apply(&after, &back), "1\n2\n3");
        assert!(move_lines(&rope, &Selection::caret(0), false).is_none());
        assert!(move_lines(&rope, &Selection::caret(4), true).is_none());
    }

    #[test]
    fn move_block_of_lines() {
        let rope = Rope::from_str("a\nb\nc\nd\n");
        let sel = Selection::new(vec![Range::new(2, 5)], 0);
        let r = move_lines(&rope, &sel, true).unwrap();
        assert_eq!(apply(&rope, &r), "a\nd\nb\nc\n");
        assert_eq!(r.selection.primary(), Range::new(4, 7));
    }

    // spec 11: "Sort lines"
    #[test]
    fn sort_selected_lines_naturally() {
        let rope = Rope::from_str("b\na10\na2");
        let sel = Selection::new(vec![Range::new(0, rope.len_chars())], 0);
        assert_eq!(
            apply(&rope, &sort_lines(&rope, &sel, false).unwrap()),
            "a2\na10\nb"
        );
        assert!(
            sort_lines(&rope, &sel, true).is_none(),
            "already descending"
        );
        let asc = Rope::from_str("a2\nb\na10");
        assert_eq!(
            apply(&asc, &sort_lines(&asc, &sel, true).unwrap()),
            "b\na10\na2"
        );
        let with_newline = Rope::from_str("b\na\n");
        assert_eq!(
            apply(
                &with_newline,
                &sort_lines(&with_newline, &Selection::caret(0), false).unwrap()
            ),
            "a\nb\n"
        );
    }

    #[test]
    fn duplicate_and_delete() {
        let rope = Rope::from_str("one\ntwo");
        let d = duplicate_lines(&rope, &Selection::caret(1)).unwrap();
        assert_eq!(apply(&rope, &d), "one\none\ntwo");
        assert_eq!(d.selection.primary().head, 5);
        let x = delete_lines(&rope, &Selection::caret(5)).unwrap();
        assert_eq!(apply(&rope, &x), "one");
        let y = delete_lines(&rope, &Selection::caret(0)).unwrap();
        assert_eq!(apply(&rope, &y), "two");
        assert_eq!(y.selection.primary().head, 0);
    }

    #[test]
    fn trim_case_insert_join() {
        let rope = Rope::from_str("a  \nb\t\nc");
        assert_eq!(
            apply(
                &rope,
                &trim_trailing_whitespace(&rope, &Selection::caret(0)).unwrap()
            ),
            "a\nb\nc"
        );
        let words = Rope::from_str("hello wORLD");
        let all = Selection::new(vec![Range::new(0, 11)], 0);
        assert_eq!(
            apply(&words, &transform_case(&words, &all, Case::Upper).unwrap()),
            "HELLO WORLD"
        );
        assert_eq!(
            apply(&words, &transform_case(&words, &all, Case::Title).unwrap()),
            "Hello World"
        );
        assert_eq!(
            apply(
                &words,
                &transform_case(&words, &Selection::caret(8), Case::Lower).unwrap()
            ),
            "hello world"
        );
        let ind = Rope::from_str("  x");
        let below = insert_line(&ind, &Selection::caret(3), true).unwrap();
        assert_eq!(apply(&ind, &below), "  x\n  ");
        assert_eq!(below.selection.primary().head, 6);
        let above = insert_line(&ind, &Selection::caret(3), false).unwrap();
        assert_eq!(apply(&ind, &above), "  \n  x");
        assert_eq!(above.selection.primary().head, 2);
        let mixed = Rope::from_str("\tx\n  \ty\n    z");
        let four = crate::edit_ops::IndentStyle {
            tab_size: 4,
            insert_spaces: true,
        };
        assert_eq!(
            apply(
                &mixed,
                &convert_indentation(&mixed, &Selection::caret(0), four).unwrap()
            ),
            "    x\n    y\n    z"
        );
        let tabs = crate::edit_ops::IndentStyle {
            insert_spaces: false,
            ..four
        };
        assert_eq!(
            apply(
                &mixed,
                &convert_indentation(&mixed, &Selection::caret(0), tabs).unwrap()
            ),
            "\tx\n\ty\n\tz"
        );
        let j = Rope::from_str("a \n   b\nc");
        assert_eq!(
            apply(&j, &join_lines(&j, &Selection::caret(0)).unwrap()),
            "a b\nc"
        );
    }
}
