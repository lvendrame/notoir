//! Line diff model for Compare (docs/specs/27 §Diff Model): hunks with line
//! ranges on both sides, intra-line word changes for modified line pairs,
//! filler rows that align the two sides, and the edit that copies a hunk from
//! one side to the other.
//!
//! A text is a list of lines split on `\n` (so `"a\n"` has two lines, the
//! second empty), matching the rope's line count.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::time::Instant;

use ropey::Rope;
use similar::{Algorithm, DiffOp};

use crate::change::ChangeSet;
use crate::display_map::line_text;

/// Lines longer than this get no intra-line diff.
pub const INLINE_MAX_CHARS: usize = 2_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffAlgorithm {
    #[default]
    Patience,
    Myers,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub ignore_trailing_whitespace: bool,
    /// Ignore every whitespace difference (implies trailing).
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub algorithm: DiffAlgorithm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Changed chars (start, end; within the line) of one modified line pair.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InlineChanges {
    pub left_line: usize,
    pub right_line: usize,
    pub left: Vec<(usize, usize)>,
    pub right: Vec<(usize, usize)>,
}

/// One change: `left` lines were replaced by `right` lines. An empty range is
/// a pure insertion (or deletion) before that line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub left: Range<usize>,
    pub right: Range<usize>,
    pub inline: Vec<InlineChanges>,
}

impl Hunk {
    pub fn lines(&self, side: Side) -> Range<usize> {
        match side {
            Side::Left => self.left.clone(),
            Side::Right => self.right.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub hunks: Vec<Hunk>,
    pub left_lines: usize,
    pub right_lines: usize,
}

fn normalize(line: &str, opts: &DiffOptions) -> u64 {
    let mut h = DefaultHasher::new();
    let s: std::borrow::Cow<str> = if opts.ignore_whitespace {
        line.chars().filter(|c| !c.is_whitespace()).collect()
    } else if opts.ignore_trailing_whitespace {
        line.trim_end().into()
    } else {
        line.into()
    };
    if opts.ignore_case {
        s.to_lowercase().hash(&mut h);
    } else {
        s.hash(&mut h);
    }
    h.finish()
}

fn algorithm(a: DiffAlgorithm) -> Algorithm {
    match a {
        DiffAlgorithm::Patience => Algorithm::Patience,
        DiffAlgorithm::Myers => Algorithm::Myers,
    }
}

/// Lines of a rope without their terminators.
pub fn rope_lines(rope: &Rope) -> Vec<String> {
    (0..rope.len_lines()).map(|l| line_text(rope, l)).collect()
}

pub fn diff_ropes(left: &Rope, right: &Rope, opts: &DiffOptions) -> Diff {
    diff_lines(&rope_lines(left), &rope_lines(right), opts, None)
}

pub fn diff_text(left: &str, right: &str, opts: &DiffOptions) -> Diff {
    let l: Vec<String> = left.split('\n').map(str::to_owned).collect();
    let r: Vec<String> = right.split('\n').map(str::to_owned).collect();
    diff_lines(&l, &r, opts, None)
}

/// Diffs two line lists. With a `deadline` the line diff may be less minimal
/// on pathological inputs.
pub fn diff_lines(
    left: &[String],
    right: &[String],
    opts: &DiffOptions,
    deadline: Option<Instant>,
) -> Diff {
    let lk: Vec<u64> = left.iter().map(|l| normalize(l, opts)).collect();
    let rk: Vec<u64> = right.iter().map(|l| normalize(l, opts)).collect();
    let ops = similar::capture_diff_slices_deadline(algorithm(opts.algorithm), &lk, &rk, deadline);
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut open: Option<(Range<usize>, Range<usize>)> = None;
    for op in ops {
        if let DiffOp::Equal { .. } = op {
            if let Some((l, r)) = open.take() {
                hunks.push(Hunk {
                    left: l,
                    right: r,
                    inline: Vec::new(),
                });
            }
            continue;
        }
        let (ol, nr) = (op.old_range(), op.new_range());
        open = Some(match open {
            Some((l, r)) => (l.start..ol.end.max(l.end), r.start..nr.end.max(r.end)),
            None => (ol, nr),
        });
    }
    if let Some((l, r)) = open {
        hunks.push(Hunk {
            left: l,
            right: r,
            inline: Vec::new(),
        });
    }
    for h in &mut hunks {
        let pairs = h.left.len().min(h.right.len());
        for i in 0..pairs {
            let (li, ri) = (h.left.start + i, h.right.start + i);
            if let Some(c) = inline(&left[li], &right[ri], opts) {
                h.inline.push(InlineChanges {
                    left_line: li,
                    right_line: ri,
                    left: c.0,
                    right: c.1,
                });
            }
        }
    }
    Diff {
        hunks,
        left_lines: left.len(),
        right_lines: right.len(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenClass {
    Word,
    Space,
    Other,
}

fn class(c: char) -> TokenClass {
    if c.is_alphanumeric() || c == '_' {
        TokenClass::Word
    } else if c.is_whitespace() {
        TokenClass::Space
    } else {
        TokenClass::Other
    }
}

/// Words, whitespace runs and single punctuation chars, with char offsets.
fn tokens(line: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut prev: Option<TokenClass> = None;
    for (i, c) in line.chars().enumerate() {
        let k = class(c);
        match out.last_mut() {
            Some((_, s)) if prev == Some(k) && k != TokenClass::Other => s.push(c),
            _ => out.push((i, c.to_string())),
        }
        prev = Some(k);
    }
    out
}

type Spans = Vec<(usize, usize)>;

/// Changed char ranges of a modified pair, or `None` when too long.
fn inline(left: &str, right: &str, opts: &DiffOptions) -> Option<(Spans, Spans)> {
    let (ln, rn) = (left.chars().count(), right.chars().count());
    if ln > INLINE_MAX_CHARS || rn > INLINE_MAX_CHARS {
        return None;
    }
    let lt = tokens(left);
    let rt = tokens(right);
    let key = |t: &String| {
        let s = if opts.ignore_whitespace && t.chars().all(char::is_whitespace) {
            String::new()
        } else {
            t.clone()
        };
        if opts.ignore_case {
            s.to_lowercase()
        } else {
            s
        }
    };
    let lk: Vec<String> = lt.iter().map(|t| key(&t.1)).collect();
    let rk: Vec<String> = rt.iter().map(|t| key(&t.1)).collect();
    let ops = similar::capture_diff_slices(Algorithm::Myers, &lk, &rk);
    let span =
        |toks: &[(usize, String)], r: Range<usize>, total: usize| -> Option<(usize, usize)> {
            if r.is_empty() {
                return None;
            }
            let a = toks[r.start].0;
            let b = toks.get(r.end).map_or(total, |t| t.0);
            Some((a, b))
        };
    let (mut l, mut r): (Spans, Spans) = (Vec::new(), Vec::new());
    let push = |v: &mut Spans, s: (usize, usize)| match v.last_mut() {
        Some(last) if last.1 == s.0 => last.1 = s.1,
        _ => v.push(s),
    };
    for op in ops {
        if let DiffOp::Equal { .. } = op {
            continue;
        }
        if let Some(s) = span(&lt, op.old_range(), ln) {
            push(&mut l, s);
        }
        if let Some(s) = span(&rt, op.new_range(), rn) {
            push(&mut r, s);
        }
    }
    Some((l, r))
}

impl Diff {
    pub fn is_identical(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Filler rows per side: (insert before this line, rows). A line equal
    /// to the side's line count means after the last line.
    pub fn fillers(&self, side: Side) -> Vec<(usize, usize)> {
        self.hunks
            .iter()
            .filter_map(|h| {
                let (mine, theirs) = (h.lines(side), h.lines(side.other()));
                (theirs.len() > mine.len()).then(|| (mine.end, theirs.len() - mine.len()))
            })
            .collect()
    }

    /// Index of the hunk containing `line` on `side`.
    pub fn hunk_at(&self, side: Side, line: usize) -> Option<usize> {
        self.hunks
            .iter()
            .position(|h| h.lines(side).contains(&line))
    }

    /// First hunk starting after `line` on `side` (or at it when `inclusive`).
    pub fn next_hunk(&self, side: Side, line: usize, inclusive: bool) -> Option<usize> {
        self.hunks.iter().position(|h| {
            let s = h.lines(side).start;
            s > line || (inclusive && s == line)
        })
    }

    /// Last hunk ending before `line` on `side`.
    pub fn prev_hunk(&self, side: Side, line: usize) -> Option<usize> {
        self.hunks.iter().rposition(|h| h.lines(side).start < line)
    }

    /// Lines of `side` that belong to a hunk, sorted (for painting).
    pub fn changed_lines(&self, side: Side) -> Vec<Range<usize>> {
        self.hunks
            .iter()
            .map(|h| h.lines(side))
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// Changed char spans (absolute line, start, end) on `side`.
    pub fn inline_spans(&self, side: Side) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for h in &self.hunks {
            for p in &h.inline {
                let (line, spans) = match side {
                    Side::Left => (p.left_line, &p.left),
                    Side::Right => (p.right_line, &p.right),
                };
                out.extend(spans.iter().map(|&(a, b)| (line, a, b)));
            }
        }
        out
    }
}

/// Char offset where `line` starts (the text length past the last line).
fn line_start(rope: &Rope, line: usize) -> usize {
    if line >= rope.len_lines() {
        rope.len_chars()
    } else {
        rope.line_to_char(line)
    }
}

/// End of `line` without its terminator.
fn line_end(rope: &Rope, line: usize) -> usize {
    line_start(rope, line) + rope.line(line).len_chars()
        - usize::from(rope.line(line).chars().last() == Some('\n'))
}

/// The edit on `to` that makes `hunk` read like `from`'s side of it (`from`
/// is `from_side` of the diff). Undoable as one change set by the caller.
pub fn copy_hunk(hunk: &Hunk, from: &Rope, from_side: Side, to: &Rope) -> ChangeSet {
    let src = hunk.lines(from_side);
    let dst = hunk.lines(from_side.other());
    let lines: Vec<String> = src.clone().map(|l| line_text(from, l)).collect();
    let joined = lines.join("\n");
    let n = to.len_lines();
    match (dst.is_empty(), src.is_empty()) {
        // Replace whole lines, keeping the terminator of the last one.
        (false, false) => {
            ChangeSet::replace(line_start(to, dst.start), line_end(to, dst.end - 1), joined)
        }
        // Insertion before line `dst.start`.
        (true, false) => {
            if dst.start < n {
                ChangeSet::insert(line_start(to, dst.start), format!("{joined}\n"))
            } else {
                ChangeSet::insert(to.len_chars(), format!("\n{joined}"))
            }
        }
        // Deletion of lines `dst`.
        (false, true) => {
            if dst.end < n {
                ChangeSet::delete(line_start(to, dst.start), line_start(to, dst.end))
            } else if dst.start > 0 {
                ChangeSet::delete(line_end(to, dst.start - 1), to.len_chars())
            } else {
                ChangeSet::delete(0, to.len_chars())
            }
        }
        (true, true) => ChangeSet::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn opts() -> DiffOptions {
        DiffOptions::default()
    }

    #[test]
    fn hunks_and_fillers() {
        let l = "The editor is fast.\nIt supports splits.\nGit is optional.";
        let r =
            "The editor is fast.\nIt supports true splits.\nAnd duplicate views.\nGit is optional.";
        let d = diff_text(l, r, &opts());
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].left, 1..2);
        assert_eq!(d.hunks[0].right, 1..3);
        assert_eq!(d.fillers(Side::Left), vec![(2, 1)]);
        assert!(d.fillers(Side::Right).is_empty());
        // "true " inserted.
        let p = &d.hunks[0].inline[0];
        assert!(p.left.is_empty());
        assert_eq!(p.right, vec![(12, 17)]);
    }

    #[test]
    fn ignore_whitespace_makes_trailing_spaces_identical() {
        let l = "alpha  \nbeta\n";
        let r = "alpha\nbeta \n";
        assert!(!diff_text(l, r, &opts()).is_identical());
        let o = DiffOptions {
            ignore_whitespace: true,
            ..opts()
        };
        assert!(diff_text(l, r, &o).is_identical());
        let o = DiffOptions {
            ignore_trailing_whitespace: true,
            ..opts()
        };
        assert!(diff_text(l, r, &o).is_identical());
        let o = DiffOptions {
            ignore_case: true,
            ..opts()
        };
        assert!(diff_text("ABC", "abc", &o).is_identical());
    }

    #[test]
    fn navigation() {
        let l = "a\nb\nc\nd\ne\nf\ng";
        let r = "a\nB\nc\nd\nE\nf\nG";
        let d = diff_text(l, r, &opts());
        assert_eq!(d.hunks.len(), 3);
        assert_eq!(d.next_hunk(Side::Right, 0, true), Some(0));
        assert_eq!(d.next_hunk(Side::Right, 1, false), Some(1));
        assert_eq!(d.prev_hunk(Side::Right, 4), Some(0));
        assert_eq!(d.hunk_at(Side::Left, 6), Some(2));
    }

    fn apply(rope: &Rope, cs: &ChangeSet) -> String {
        let mut r = rope.clone();
        cs.apply(&mut r);
        r.to_string()
    }

    #[test]
    fn copy_each_hunk_kind() {
        let cases = [
            ("a\nb\nc", "a\nX\nc"),
            ("a\nc", "a\nb\nc"),
            ("a\nb\nc", "a\nc"),
            ("a\nX", "a\nY\nz"),
            ("a\nb\n", "a\n"),
            ("a", "a\nb"),
            ("a\nb", "a"),
            ("", "x\ny"),
            ("x\ny", ""),
        ];
        for (l, r) in cases {
            let (lr, rr) = (Rope::from_str(l), Rope::from_str(r));
            let d = diff_text(l, r, &opts());
            // Copy every hunk left → right (last first keeps line numbers valid).
            let mut to = rr.clone();
            for h in d.hunks.iter().rev() {
                let cs = copy_hunk(h, &lr, Side::Left, &to);
                cs.apply(&mut to);
            }
            assert_eq!(to.to_string(), l, "{l:?} → {r:?}");
            let mut to = lr.clone();
            for h in d.hunks.iter().rev() {
                let cs = copy_hunk(h, &rr, Side::Right, &to);
                to = Rope::from_str(&apply(&to, &cs));
            }
            assert_eq!(to.to_string(), r, "{r:?} → {l:?}");
        }
    }

    proptest! {
        #[test]
        fn copying_all_hunks_reproduces_the_source(l in "[ab\n]{0,24}", r in "[ab\n]{0,24}") {
            let (lr, rr) = (Rope::from_str(&l), Rope::from_str(&r));
            let d = diff_ropes(&lr, &rr, &opts());
            let mut to = rr.clone();
            for h in d.hunks.iter().rev() {
                let cs = copy_hunk(h, &lr, Side::Left, &to);
                cs.apply(&mut to);
            }
            prop_assert_eq!(to.to_string(), l);
        }
    }
}
