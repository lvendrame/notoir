//! Selections: one or more ranges (multi-cursor ready), char offsets.

use crate::change::{Assoc, ChangeSet};

/// A caret (`anchor == head`) or a selected range; `head` is where the caret is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
}

impl Range {
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    pub fn caret(pos: usize) -> Self {
        Self::new(pos, pos)
    }

    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn map(&self, changes: &ChangeSet) -> Range {
        // Range edges stay outside text inserted at them; carets move along.
        if self.is_empty() {
            let p = changes.map_pos(self.head, Assoc::After);
            return Range::caret(p);
        }
        let (from_assoc, to_assoc) = (Assoc::After, Assoc::Before);
        let from = changes.map_pos(self.from(), from_assoc);
        let to = changes.map_pos(self.to(), to_assoc).max(from);
        if self.anchor <= self.head {
            Range::new(from, to)
        } else {
            Range::new(to, from)
        }
    }

    fn clamp(&self, len: usize) -> Range {
        Range::new(self.anchor.min(len), self.head.min(len))
    }
}

/// Non-empty list of ranges, sorted and non-overlapping, with a primary index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    ranges: Vec<Range>,
    primary: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Self::caret(0)
    }
}

impl Selection {
    pub fn caret(pos: usize) -> Self {
        Self::single(Range::caret(pos))
    }

    pub fn single(range: Range) -> Self {
        Self {
            ranges: vec![range],
            primary: 0,
        }
    }

    /// Builds a normalized selection: sorted, overlapping ranges merged.
    /// Falls back to a caret at 0 when `ranges` is empty.
    pub fn new(ranges: Vec<Range>, primary: usize) -> Self {
        if ranges.is_empty() {
            return Self::default();
        }
        let primary_range = ranges[primary.min(ranges.len() - 1)];
        let mut sorted = ranges;
        sorted.sort_by_key(|r| r.from());
        let mut merged: Vec<Range> = Vec::with_capacity(sorted.len());
        for r in sorted {
            match merged.last_mut() {
                Some(last)
                    if r.from() < last.to()
                        || (r.from() == last.to() && (r.is_empty() || last.is_empty())) =>
                {
                    let (from, to) = (last.from(), last.to().max(r.to()));
                    *last = if last.anchor <= last.head {
                        Range::new(from, to)
                    } else {
                        Range::new(to, from)
                    };
                }
                _ => merged.push(r),
            }
        }
        let primary = merged
            .iter()
            .position(|r| r.from() <= primary_range.head && primary_range.head <= r.to())
            .unwrap_or(0);
        Self {
            ranges: merged,
            primary,
        }
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.iter().all(Range::is_empty)
    }

    /// Transforms every range through `changes` (edits from any origin).
    pub fn map(&self, changes: &ChangeSet) -> Selection {
        let ranges = self.ranges.iter().map(|r| r.map(changes)).collect();
        Selection::new(ranges, self.primary)
    }

    /// Clamps every range into a document of `len` chars.
    pub fn clamp(&self, len: usize) -> Selection {
        let ranges = self.ranges.iter().map(|r| r.clamp(len)).collect();
        Selection::new(ranges, self.primary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_moves_with_insert_before_it() {
        let sel = Selection::caret(10);
        let mapped = sel.map(&ChangeSet::insert(0, "abc\n"));
        assert_eq!(mapped.primary(), Range::caret(14));
    }

    #[test]
    fn caret_inside_deleted_text_collapses() {
        let sel = Selection::caret(5);
        assert_eq!(sel.map(&ChangeSet::delete(2, 8)).primary(), Range::caret(2));
    }

    #[test]
    fn range_does_not_grow_from_inserts_at_its_edges() {
        let sel = Selection::single(Range::new(2, 4));
        let cs = ChangeSet::new(vec![
            crate::change::Edit::new(2, 2, "X"),
            crate::change::Edit::new(4, 4, "Y"),
        ])
        .unwrap();
        assert_eq!(sel.map(&cs).primary(), Range::new(3, 5));
    }

    #[test]
    fn overlapping_ranges_merge_and_keep_primary() {
        let sel = Selection::new(
            vec![Range::new(0, 3), Range::new(10, 12), Range::new(2, 5)],
            1,
        );
        assert_eq!(sel.ranges(), &[Range::new(0, 5), Range::new(10, 12)]);
        assert_eq!(sel.primary(), Range::new(10, 12));
    }

    #[test]
    fn backward_range_keeps_direction() {
        let sel = Selection::single(Range::new(8, 4));
        let mapped = sel.map(&ChangeSet::insert(0, "ab"));
        assert_eq!(mapped.primary(), Range::new(10, 6));
    }
}
