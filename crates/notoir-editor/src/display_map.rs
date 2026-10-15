//! Buffer lines ↔ display rows (word wrap and folded lines). Row counts per
//! line plus prefix sums; edits update only the touched lines.

use ropey::Rope;

use crate::change::ChangeSet;
use crate::wrap::wrap_line;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapConfig {
    /// Columns per row (≥ 1).
    pub cols: usize,
    pub tab: usize,
}

/// Position of a display row: buffer line and the row index within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowRef {
    pub line: usize,
    pub sub_row: usize,
}

#[derive(Clone, Debug)]
pub struct DisplayMap {
    wrap: Option<WrapConfig>,
    /// Rows per buffer line (only when wrapping).
    rows: Vec<u32>,
    /// `prefix[i]` = rows before line `i`; len = lines + 1. Valid up to
    /// `prefix_valid` (exclusive); recomputed lazily.
    prefix: Vec<u64>,
    prefix_valid: usize,
    lines: usize,
    /// Hidden (folded) buffer lines: sorted, disjoint, inclusive ranges.
    hidden: Vec<(usize, usize)>,
}

/// Line text without its trailing newline.
pub fn line_text(rope: &Rope, line: usize) -> String {
    let mut s: String = rope.line(line).chars().collect();
    if s.ends_with('\n') {
        s.pop();
    }
    s
}

fn rows_of(rope: &Rope, line: usize, cfg: WrapConfig) -> u32 {
    wrap_line(&line_text(rope, line), cfg.cols, cfg.tab).len() as u32
}

impl DisplayMap {
    pub fn new(rope: &Rope, wrap: Option<WrapConfig>) -> Self {
        let lines = rope.len_lines();
        let rows = match wrap {
            Some(cfg) => (0..lines).map(|l| rows_of(rope, l, cfg)).collect(),
            None => Vec::new(),
        };
        let mut map = Self {
            wrap,
            rows,
            prefix: vec![0],
            prefix_valid: 1,
            lines,
            hidden: Vec::new(),
        };
        map.ensure_prefix(lines);
        map
    }

    pub fn wrap(&self) -> Option<WrapConfig> {
        self.wrap
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    /// Whether rows need the prefix table (wrapping or folds).
    fn indexed(&self) -> bool {
        self.wrap.is_some() || !self.hidden.is_empty()
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        let i = self.hidden.partition_point(|r| r.1 < line);
        self.hidden.get(i).is_some_and(|r| r.0 <= line)
    }

    /// Replaces the hidden line ranges (inclusive; merged and sorted here).
    pub fn set_hidden(&mut self, ranges: &[(usize, usize)]) {
        let mut v: Vec<(usize, usize)> = ranges
            .iter()
            .filter(|r| r.0 <= r.1 && r.0 < self.lines)
            .map(|r| (r.0, r.1.min(self.lines.saturating_sub(1))))
            .collect();
        v.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(v.len());
        for r in v {
            match merged.last_mut() {
                Some(last) if r.0 <= last.1 + 1 => last.1 = last.1.max(r.1),
                _ => merged.push(r),
            }
        }
        if merged == self.hidden {
            return;
        }
        let first = merged
            .iter()
            .chain(self.hidden.iter())
            .map(|r| r.0)
            .min()
            .unwrap_or(0);
        let was_indexed = self.indexed();
        self.hidden = merged;
        if !was_indexed {
            self.prefix.truncate(1);
            self.prefix_valid = 1;
        } else {
            self.prefix_valid = self.prefix_valid.min(first + 1);
        }
    }

    /// Rows of `line` ignoring folds.
    fn raw_rows(&self, line: usize) -> u32 {
        match self.wrap {
            None => 1,
            Some(_) => self.rows.get(line).copied().unwrap_or(1),
        }
    }

    /// Updates the map for `changes`, which turned `before` into `after`.
    pub fn apply(&mut self, before: &Rope, after: &Rope, changes: &ChangeSet) {
        let old_lines = self.lines;
        self.lines = after.len_lines();
        if self.wrap.is_none() && !self.hidden.is_empty() {
            // Hidden ranges are re-set by the owner after edits; invalidate
            // from the first changed line.
            let first = changes
                .edits()
                .first()
                .map_or(0, |e| before.char_to_line(e.start.min(before.len_chars())));
            self.hidden.retain(|r| r.0 < self.lines);
            for r in &mut self.hidden {
                r.1 = r.1.min(self.lines - 1);
            }
            if old_lines != self.lines || first < self.prefix_valid {
                self.prefix_valid = self.prefix_valid.min(first + 1);
            }
            return;
        }
        let Some(cfg) = self.wrap else { return };
        let mut char_delta: isize = 0;
        let mut first_dirty = usize::MAX;
        for e in changes.edits() {
            let old_l0 = before.char_to_line(e.start);
            let old_l1 = before.char_to_line(e.end);
            let new_start = (e.start as isize + char_delta) as usize;
            let new_end = new_start + e.inserted_len();
            let new_l0 = after.char_to_line(new_start);
            let new_l1 = after.char_to_line(new_end.min(after.len_chars()));
            char_delta += e.inserted_len() as isize - e.deleted_len() as isize;
            let replaced = old_l1 - old_l0 + 1;
            let fresh: Vec<u32> = (new_l0..=new_l1).map(|l| rows_of(after, l, cfg)).collect();
            let end = (new_l0 + replaced).min(self.rows.len());
            self.rows.splice(new_l0..end, fresh);
            first_dirty = first_dirty.min(new_l0);
        }
        debug_assert_eq!(self.rows.len(), self.lines, "row table out of sync");
        if self.rows.len() != self.lines {
            *self = Self::new(after, self.wrap);
            return;
        }
        if first_dirty != usize::MAX {
            self.prefix_valid = self.prefix_valid.min(first_dirty + 1);
        }
    }

    fn ensure_prefix(&mut self, upto_line: usize) {
        if !self.indexed() {
            return;
        }
        let upto = upto_line.min(self.lines);
        self.prefix.truncate(self.prefix_valid.max(1));
        while self.prefix.len() <= upto {
            let i = self.prefix.len() - 1;
            let rows = if self.is_hidden(i) {
                0
            } else {
                self.raw_rows(i)
            };
            let next = self.prefix[i] + u64::from(rows);
            self.prefix.push(next);
        }
        self.prefix_valid = self.prefix.len();
    }

    pub fn total_rows(&mut self) -> usize {
        if !self.indexed() {
            return self.lines;
        }
        self.ensure_prefix(self.lines);
        self.prefix[self.lines] as usize
    }

    /// Display rows of `line` (0 when folded away).
    pub fn rows_in_line(&self, line: usize) -> usize {
        if self.is_hidden(line) {
            0
        } else {
            self.raw_rows(line) as usize
        }
    }

    /// First display row of `line` (a hidden line maps to the row after its
    /// fold header).
    pub fn line_to_row(&mut self, line: usize) -> usize {
        if !self.indexed() {
            return line.min(self.lines);
        }
        let line = line.min(self.lines);
        self.ensure_prefix(line);
        self.prefix[line] as usize
    }

    /// Line and sub-row of display row `row` (clamped to the last row).
    /// Never a hidden line unless every line is hidden.
    pub fn row_to_line(&mut self, row: usize) -> RowRef {
        if !self.indexed() {
            return RowRef {
                line: row.min(self.lines.saturating_sub(1)),
                sub_row: 0,
            };
        }
        let total = self.total_rows();
        let row = row.min(total.saturating_sub(1)) as u64;
        // Last line whose first row is ≤ row.
        let mut line = self.prefix[..=self.lines]
            .partition_point(|&p| p <= row)
            .saturating_sub(1)
            .min(self.lines.saturating_sub(1));
        // Trailing hidden lines share the last prefix: step back to a visible one.
        while line > 0 && self.is_hidden(line) {
            line -= 1;
        }
        RowRef {
            line,
            sub_row: row.saturating_sub(self.prefix[line]) as usize,
        }
    }

    /// Char offsets (within the line) where the line's rows start.
    pub fn row_starts(&self, rope: &Rope, line: usize) -> Vec<usize> {
        match self.wrap {
            None => vec![0],
            Some(cfg) => wrap_line(&line_text(rope, line), cfg.cols, cfg.tab),
        }
    }

    /// Display row containing char `pos` (a caret at a wrap boundary belongs
    /// to the following row).
    pub fn row_of_char(&mut self, rope: &Rope, pos: usize) -> usize {
        let pos = pos.min(rope.len_chars());
        let line = rope.char_to_line(pos);
        let col_chars = pos - rope.line_to_char(line);
        let starts = self.row_starts(rope, line);
        let sub = starts
            .partition_point(|&s| s <= col_chars)
            .saturating_sub(1);
        self.line_to_row(line) + sub
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn cfg(cols: usize) -> Option<WrapConfig> {
        Some(WrapConfig { cols, tab: 4 })
    }

    #[test]
    fn without_wrap_rows_are_lines() {
        let rope = Rope::from_str("a\nb\nc");
        let mut m = DisplayMap::new(&rope, None);
        assert_eq!(m.total_rows(), 3);
        assert_eq!(
            m.row_to_line(2),
            RowRef {
                line: 2,
                sub_row: 0
            }
        );
    }

    #[test]
    fn wrapped_rows_map_both_ways() {
        let rope = Rope::from_str("short\nhello world foo bar\nx");
        let mut m = DisplayMap::new(&rope, cfg(6));
        assert_eq!(m.rows_in_line(1), 4);
        assert_eq!(m.total_rows(), 6);
        assert_eq!(m.line_to_row(2), 5);
        assert_eq!(
            m.row_to_line(3),
            RowRef {
                line: 1,
                sub_row: 2
            }
        );
        assert_eq!(
            m.row_to_line(99),
            RowRef {
                line: 2,
                sub_row: 0
            }
        );
        let pos = rope.line_to_char(1) + 6; // start of "world"
        assert_eq!(m.row_of_char(&rope, pos), 2);
    }

    #[test]
    fn hidden_lines_have_no_rows() {
        let rope = Rope::from_str("0\n1\n2\n3\n4\n5");
        let mut m = DisplayMap::new(&rope, None);
        m.set_hidden(&[(1, 3)]);
        assert_eq!(m.total_rows(), 3);
        assert_eq!(m.row_to_line(1).line, 4);
        assert_eq!(m.line_to_row(5), 2);
        assert_eq!(m.rows_in_line(2), 0);
        // Trailing fold.
        m.set_hidden(&[(4, 5)]);
        assert_eq!(m.total_rows(), 4);
        assert_eq!(m.row_to_line(9).line, 3);
        // With wrap.
        let rope = Rope::from_str("short\nhello world foo bar\nx\ny");
        let mut m = DisplayMap::new(&rope, cfg(6));
        m.set_hidden(&[(1, 1)]);
        assert_eq!(m.total_rows(), 3);
        assert_eq!(m.row_to_line(1).line, 2);
        m.set_hidden(&[]);
        assert_eq!(m.total_rows(), 7);
    }

    fn arb_edit() -> impl Strategy<Value = (String, usize, usize, String)> {
        (
            "[a-z \n]{0,60}",
            any::<usize>(),
            0..8usize,
            "[a-z \n]{0,12}",
        )
    }

    proptest! {
        #[test]
        fn incremental_update_matches_rebuild((text, at, del, ins) in arb_edit(), cols in 1..12usize) {
            let before = Rope::from_str(&text);
            let len = before.len_chars();
            let start = if len == 0 { 0 } else { at % (len + 1) };
            let end = (start + del).min(len);
            let cs = ChangeSet::replace(start, end, ins.as_str());
            let mut after = before.clone();
            cs.apply(&mut after);
            let mut m = DisplayMap::new(&before, cfg(cols));
            m.apply(&before, &after, &cs);
            let mut fresh = DisplayMap::new(&after, cfg(cols));
            prop_assert_eq!(m.total_rows(), fresh.total_rows());
            for l in 0..after.len_lines() {
                prop_assert_eq!(m.line_to_row(l), fresh.line_to_row(l));
            }
        }
    }
}
