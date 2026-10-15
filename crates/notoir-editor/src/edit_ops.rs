//! Editing operations as pure functions: (rope, selection) → change set and
//! new selection (docs/specs/11). Every range of a multi-range selection is
//! edited; the resulting carets are mapped through the change.

use ropey::Rope;

use crate::buffer::EditKind;
use crate::change::{ChangeSet, Edit};
use crate::display_map::line_text;
use crate::motion::{next_grapheme, next_word, prev_grapheme, prev_word};
use crate::selection::{Range, Selection};

#[derive(Clone, Debug)]
pub struct EditResult {
    pub changes: ChangeSet,
    pub selection: Selection,
    pub kind: EditKind,
}

/// `editor.tabSize` / `editor.insertSpaces`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndentStyle {
    pub tab_size: usize,
    pub insert_spaces: bool,
}

impl IndentStyle {
    fn unit(&self) -> String {
        if self.insert_spaces {
            " ".repeat(self.tab_size.max(1))
        } else {
            "\t".to_owned()
        }
    }
}

/// Builds the result from one edit per range; each range's caret lands at the
/// end of its replacement.
fn per_range(
    sel: &Selection,
    kind: EditKind,
    f: impl Fn(&Range) -> Option<Edit>,
) -> Option<EditResult> {
    let mut edits = Vec::new();
    for r in sel.ranges() {
        if let Some(e) = f(r) {
            edits.push(e);
        }
    }
    if edits.is_empty() {
        return None;
    }
    // Carets: after each edit's inserted text, in post-change coordinates.
    let mut sorted = edits.clone();
    sorted.sort_by_key(|e| e.start);
    let mut delta: isize = 0;
    let carets: Vec<Range> = sorted
        .iter()
        .map(|e| {
            let start = (e.start as isize + delta) as usize;
            delta += e.inserted_len() as isize - e.deleted_len() as isize;
            Range::caret(start + e.inserted_len())
        })
        .collect();
    let changes = ChangeSet::new(edits).ok()?;
    Some(EditResult {
        changes,
        selection: Selection::new(carets, sel.primary_index()),
        kind,
    })
}

/// Types or pastes `text` over every range.
pub fn insert(sel: &Selection, text: &str, kind: EditKind) -> Option<EditResult> {
    per_range(sel, kind, |r| Some(Edit::new(r.from(), r.to(), text)))
}

/// Pastes `texts[i]` over the i-th range (multi-cursor paste of as many
/// lines as there are cursors).
pub fn insert_each(sel: &Selection, texts: &[String], kind: EditKind) -> Option<EditResult> {
    if texts.len() != sel.len() {
        return None;
    }
    let next = std::cell::Cell::new(0);
    per_range(sel, kind, |r| {
        let i = next.get();
        next.set(i + 1);
        Some(Edit::new(r.from(), r.to(), texts[i].as_str()))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Grapheme,
    Word,
}

/// Backspace: deletes the selection, or the grapheme/word before each caret.
pub fn delete_backward(rope: &Rope, sel: &Selection, unit: Unit) -> Option<EditResult> {
    per_range(sel, EditKind::DeleteBackward, |r| {
        if !r.is_empty() {
            return Some(Edit::new(r.from(), r.to(), ""));
        }
        let from = match unit {
            Unit::Grapheme => prev_grapheme(rope, r.head),
            Unit::Word => prev_word(rope, r.head),
        };
        (from < r.head).then(|| Edit::new(from, r.head, ""))
    })
}

/// Delete: deletes the selection, or the grapheme/word after each caret.
pub fn delete_forward(rope: &Rope, sel: &Selection, unit: Unit) -> Option<EditResult> {
    per_range(sel, EditKind::DeleteForward, |r| {
        if !r.is_empty() {
            return Some(Edit::new(r.from(), r.to(), ""));
        }
        let to = match unit {
            Unit::Grapheme => next_grapheme(rope, r.head),
            Unit::Word => next_word(rope, r.head),
        };
        (to > r.head).then(|| Edit::new(r.head, to, ""))
    })
}

/// Markdown list marker at the start of `line`: (indent + marker text incl.
/// trailing space and task box, next marker for continuation).
fn list_marker(line: &str) -> Option<(usize, String)> {
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let rest = &line[indent.len()..];
    let (marker, after) = if let Some(r) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        (rest[..2].to_owned(), r)
    } else {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let tail = &rest[digits.len()..];
        if digits.is_empty()
            || digits.len() > 9
            || !(tail.starts_with(". ") || tail.starts_with(") "))
        {
            return None;
        }
        let n: u64 = digits.parse().ok()?;
        (format!("{}{} ", n + 1, &tail[..1]), &tail[2..])
    };
    let task = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find(|t| after.starts_with(**t));
    let used =
        indent.chars().count() + marker.chars().count() + task.map_or(0, |t| t.chars().count());
    let next = format!(
        "{indent}{marker}{}",
        if task.is_some() { "[ ] " } else { "" }
    );
    Some((used, next))
}

/// Enter: newline + the current line's indentation; in Markdown, continues
/// list items and removes the marker of an empty item.
pub fn newline(rope: &Rope, sel: &Selection, markdown_lists: bool) -> Option<EditResult> {
    per_range(sel, EditKind::Other, |r| {
        let line = rope.char_to_line(r.from());
        let start = rope.line_to_char(line);
        let text = line_text(rope, line);
        let indent: String = text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if markdown_lists && let Some((used, next)) = list_marker(&text) {
            let at_end = r.to() == start + text.chars().count();
            if text.chars().count() == used && at_end {
                // Empty item: Enter ends the list.
                return Some(Edit::new(start, r.to(), ""));
            }
            return Some(Edit::new(r.from(), r.to(), format!("\n{next}")));
        }
        let indent: String = indent.chars().take(r.from() - start).collect();
        Some(Edit::new(r.from(), r.to(), format!("\n{indent}")))
    })
}

fn lines_of(rope: &Rope, r: &Range) -> std::ops::RangeInclusive<usize> {
    let first = rope.char_to_line(r.from());
    let mut last = rope.char_to_line(r.to());
    // A selection ending at column 0 does not include that line.
    if last > first && rope.line_to_char(last) == r.to() {
        last -= 1;
    }
    first..=last
}

/// Tab: inserts indentation at carets, or indents every line of multi-line
/// selections.
pub fn indent(rope: &Rope, sel: &Selection, style: IndentStyle) -> Option<EditResult> {
    let multi_line = sel
        .ranges()
        .iter()
        .any(|r| !r.is_empty() && rope.char_to_line(r.from()) != rope.char_to_line(r.to()));
    if !multi_line {
        return per_range(sel, EditKind::Other, |r| {
            let unit = if style.insert_spaces {
                let line = rope.char_to_line(r.from());
                let col: usize = crate::wrap::col_of(
                    &line_text(rope, line),
                    r.from() - rope.line_to_char(line),
                    style.tab_size,
                );
                " ".repeat(style.tab_size.max(1) - col % style.tab_size.max(1))
            } else {
                "\t".to_owned()
            };
            Some(Edit::new(r.from(), r.to(), unit))
        });
    }
    let mut lines: Vec<usize> = sel
        .ranges()
        .iter()
        .flat_map(|r| lines_of(rope, r))
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let edits: Vec<Edit> = lines
        .iter()
        .filter(|l| !line_text(rope, **l).is_empty())
        .map(|l| {
            let at = rope.line_to_char(*l);
            Edit::new(at, at, style.unit())
        })
        .collect();
    shift_lines(sel, edits)
}

/// Shift+Tab: removes one indentation unit from every touched line.
pub fn outdent(rope: &Rope, sel: &Selection, style: IndentStyle) -> Option<EditResult> {
    let mut lines: Vec<usize> = sel
        .ranges()
        .iter()
        .flat_map(|r| lines_of(rope, r))
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let edits: Vec<Edit> = lines
        .iter()
        .filter_map(|l| {
            let text = line_text(rope, *l);
            let n = if text.starts_with('\t') {
                1
            } else {
                text.chars()
                    .take(style.tab_size.max(1))
                    .take_while(|c| *c == ' ')
                    .count()
            };
            (n > 0).then(|| {
                let at = rope.line_to_char(*l);
                Edit::new(at, at + n, "")
            })
        })
        .collect();
    shift_lines(sel, edits)
}

/// Applies line-start edits keeping the selection over the same text.
fn shift_lines(sel: &Selection, edits: Vec<Edit>) -> Option<EditResult> {
    if edits.is_empty() {
        return None;
    }
    let changes = ChangeSet::new(edits).ok()?;
    let ranges = sel
        .ranges()
        .iter()
        .map(|r| {
            let map = |p: usize, other: usize| {
                changes.map_pos(
                    p,
                    if p <= other {
                        crate::Assoc::Before
                    } else {
                        crate::Assoc::After
                    },
                )
            };
            Range::new(map(r.anchor, r.head), map(r.head, r.anchor))
        })
        .collect();
    Some(EditResult {
        changes,
        selection: Selection::new(ranges, sel.primary_index()),
        kind: EditKind::Other,
    })
}

/// Text of every range joined by newlines (copy/cut).
pub fn selected_text(rope: &Rope, sel: &Selection) -> String {
    sel.ranges()
        .iter()
        .filter(|r| !r.is_empty())
        .map(|r| rope.slice(r.from()..r.to()).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ranges of whole lines under empty carets (copy/cut without a selection,
/// `editor.emptySelectionClipboard`).
pub fn line_ranges(rope: &Rope, sel: &Selection) -> Selection {
    let ranges = sel
        .ranges()
        .iter()
        .map(|r| {
            let line = rope.char_to_line(r.head);
            let start = rope.line_to_char(line);
            let end = if line + 1 < rope.len_lines() {
                rope.line_to_char(line + 1)
            } else {
                rope.len_chars()
            };
            Range::new(start, end)
        })
        .collect();
    Selection::new(ranges, sel.primary_index())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, res: &EditResult) -> String {
        let mut r = Rope::from_str(text);
        res.changes.apply(&mut r);
        r.to_string()
    }

    const SPACES: IndentStyle = IndentStyle {
        tab_size: 4,
        insert_spaces: true,
    };

    #[test]
    fn typing_replaces_selections_and_moves_carets() {
        let sel = Selection::new(vec![Range::new(0, 1), Range::caret(3)], 0);
        let res = insert(&sel, "XY", EditKind::Insert).unwrap();
        assert_eq!(apply("abcd", &res), "XYbcXYd");
        assert_eq!(res.selection.ranges(), &[Range::caret(2), Range::caret(6)]);
    }

    // spec 11: "Backspace deletes a whole emoji"
    #[test]
    fn backspace_deletes_whole_grapheme() {
        let rope = Rope::from_str("a👍🏽");
        let res = delete_backward(&rope, &Selection::caret(3), Unit::Grapheme).unwrap();
        assert_eq!(apply("a👍🏽", &res), "a");
        assert!(delete_backward(&rope, &Selection::caret(0), Unit::Grapheme).is_none());
    }

    #[test]
    fn word_deletes() {
        let rope = Rope::from_str("foo bar baz");
        let res = delete_backward(&rope, &Selection::caret(7), Unit::Word).unwrap();
        assert_eq!(apply("foo bar baz", &res), "foo  baz");
        let res = delete_forward(&rope, &Selection::caret(3), Unit::Word).unwrap();
        assert_eq!(apply("foo bar baz", &res), "foo baz");
    }

    #[test]
    fn enter_keeps_indentation() {
        let rope = Rope::from_str("    fn x() {");
        let res = newline(&rope, &Selection::caret(12), false).unwrap();
        assert_eq!(apply("    fn x() {", &res), "    fn x() {\n    ");
        assert_eq!(res.selection.primary(), Range::caret(17));
    }

    // spec 11: "Enter continues a task list"
    #[test]
    fn enter_continues_markdown_lists() {
        let text = "- [ ] buy milk";
        let rope = Rope::from_str(text);
        let res = newline(&rope, &Selection::caret(14), true).unwrap();
        assert_eq!(apply(text, &res), "- [ ] buy milk\n- [ ] ");
        assert_eq!(res.selection.primary(), Range::caret(21));

        let rope = Rope::from_str("9. nine");
        assert_eq!(
            apply(
                "9. nine",
                &newline(&rope, &Selection::caret(7), true).unwrap()
            ),
            "9. nine\n10. "
        );

        let rope = Rope::from_str("  - ");
        assert_eq!(
            apply("  - ", &newline(&rope, &Selection::caret(4), true).unwrap()),
            "",
            "empty item ends the list"
        );
    }

    #[test]
    fn tab_indents_to_next_stop_or_lines() {
        let rope = Rope::from_str("ab");
        assert_eq!(
            apply("ab", &indent(&rope, &Selection::caret(1), SPACES).unwrap()),
            "a   b"
        );
        let text = "one\ntwo\n\nthree";
        let rope = Rope::from_str(text);
        let res = indent(&rope, &Selection::single(Range::new(0, 14)), SPACES).unwrap();
        assert_eq!(apply(text, &res), "    one\n    two\n\n    three");
        assert_eq!(res.selection.primary(), Range::new(0, 26));
        let tabs = IndentStyle {
            tab_size: 4,
            insert_spaces: false,
        };
        assert_eq!(
            apply("ab", &indent(&rope, &Selection::caret(1), tabs).unwrap()),
            "a\tb"
        );
    }

    #[test]
    fn shift_tab_outdents() {
        let text = "    one\n  two\n\tthree";
        let rope = Rope::from_str(text);
        let res = outdent(
            &rope,
            &Selection::single(Range::new(0, rope.len_chars())),
            SPACES,
        )
        .unwrap();
        assert_eq!(apply(text, &res), "one\ntwo\nthree");
    }

    #[test]
    fn clipboard_helpers() {
        let rope = Rope::from_str("one\ntwo");
        assert_eq!(
            selected_text(&rope, &Selection::single(Range::new(1, 6))),
            "ne\ntw"
        );
        let lines = line_ranges(&rope, &Selection::caret(1));
        assert_eq!(selected_text(&rope, &lines), "one\n");
    }
}
