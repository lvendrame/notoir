//! Buffer lines ↔ display rows (word wrap now, folds later). Row counts per
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

    /// Updates the map for `changes`, which turned `before` into `after`.
    pub fn apply(&mut self, before: &Rope, after: &Rope, changes: &ChangeSet) {
        self.lines = after.len_lines();
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
        if self.wrap.is_none() {
            return;
        }
        let upto = upto_line.min(self.lines);
        self.prefix.truncate(self.prefix_valid);
        while self.prefix.len() <= upto {
            let i = self.prefix.len() - 1;
            let next = self.prefix[i] + u64::from(self.rows[i]);
            self.prefix.push(next);
        }
        self.prefix_valid = self.prefix.len();
    }

    pub fn total_rows(&mut self) -> usize {
        match self.wrap {
            None => self.lines,
            Some(_) => {
                self.ensure_prefix(self.lines);
                self.prefix[self.lines] as usize
            }
        }
    }

    pub fn rows_in_line(&self, line: usize) -> usize {
        match self.wrap {
            None => 1,
            Some(_) => self.rows.get(line).copied().unwrap_or(1) as usize,
        }
    }

    /// First display row of `line`.
    pub fn line_to_row(&mut self, line: usize) -> usize {
        match self.wrap {
            None => line.min(self.lines),
            Some(_) => {
                let line = line.min(self.lines);
                self.ensure_prefix(line);
                self.prefix[line] as usize
            }
        }
    }

    /// Line and sub-row of display row `row` (clamped to the last row).
    pub fn row_to_line(&mut self, row: usize) -> RowRef {
        match self.wrap {
            None => RowRef {
                line: row.min(self.lines.saturating_sub(1)),
                sub_row: 0,
            },
            Some(_) => {
                let total = self.total_rows();
                let row = row.min(total.saturating_sub(1)) as u64;
                // Last line whose first row is ≤ row.
                let line = self.prefix[..=self.lines]
                    .partition_point(|&p| p <= row)
                    .saturating_sub(1);
                let line = line.min(self.lines.saturating_sub(1));
                RowRef {
                    line,
                    sub_row: (row - self.prefix[line]) as usize,
                }
            }
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
