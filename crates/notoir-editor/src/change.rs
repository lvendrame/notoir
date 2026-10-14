//! Change sets: atomic groups of non-overlapping edits expressed in the
//! coordinates of the text *before* the change (char offsets into the rope).

use std::sync::Arc;

use ropey::Rope;

/// One replacement of `start..end` (pre-change char offsets) by `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: Arc<str>,
}

impl Edit {
    pub fn new(start: usize, end: usize, text: impl Into<Arc<str>>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }

    pub fn inserted_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn deleted_len(&self) -> usize {
        self.end - self.start
    }
}

/// Which side a position sticks to when text is inserted exactly at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assoc {
    Before,
    After,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChangeError {
    #[error("edit {start}..{end} is reversed")]
    Reversed { start: usize, end: usize },
    #[error("edits overlap at {0}")]
    Overlap(usize),
    #[error("edit end {end} is past the document length {len}")]
    OutOfBounds { end: usize, len: usize },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    edits: Vec<Edit>,
}

impl ChangeSet {
    /// Builds a change set; edits are sorted by start. Several pure inserts at
    /// the same position are kept in the given order.
    pub fn new(mut edits: Vec<Edit>) -> Result<Self, ChangeError> {
        for e in &edits {
            if e.start > e.end {
                return Err(ChangeError::Reversed {
                    start: e.start,
                    end: e.end,
                });
            }
        }
        edits.retain(|e| e.start != e.end || !e.text.is_empty());
        edits.sort_by_key(|e| e.start);
        for pair in edits.windows(2) {
            if pair[0].end > pair[1].start {
                return Err(ChangeError::Overlap(pair[1].start));
            }
        }
        Ok(Self { edits })
    }

    pub fn insert(pos: usize, text: impl Into<Arc<str>>) -> Self {
        Self::single(Edit::new(pos, pos, text))
    }

    pub fn delete(start: usize, end: usize) -> Self {
        Self::single(Edit::new(start, end, ""))
    }

    pub fn replace(start: usize, end: usize, text: impl Into<Arc<str>>) -> Self {
        Self::single(Edit::new(start, end, text))
    }

    fn single(edit: Edit) -> Self {
        Self::new(vec![edit]).unwrap_or_default()
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Fails if any edit reaches past `len` chars.
    pub fn check_bounds(&self, len: usize) -> Result<(), ChangeError> {
        match self.edits.last() {
            Some(e) if e.end > len => Err(ChangeError::OutOfBounds { end: e.end, len }),
            _ => Ok(()),
        }
    }

    /// Applies the edits to `rope` (caller checked bounds).
    pub fn apply(&self, rope: &mut Rope) {
        for e in self.edits.iter().rev() {
            if e.end > e.start {
                rope.remove(e.start..e.end);
            }
            if !e.text.is_empty() {
                rope.insert(e.start, &e.text);
            }
        }
    }

    /// The change set that undoes `self`, given the text before `self` applied.
    pub fn invert(&self, before: &Rope) -> ChangeSet {
        let mut delta: isize = 0;
        let edits = self
            .edits
            .iter()
            .map(|e| {
                let start = (e.start as isize + delta) as usize;
                let end = start + e.inserted_len();
                delta += e.inserted_len() as isize - e.deleted_len() as isize;
                Edit::new(start, end, before.slice(e.start..e.end).to_string())
            })
            .collect();
        ChangeSet { edits }
    }

    /// Maps a pre-change position to its post-change position.
    pub fn map_pos(&self, pos: usize, assoc: Assoc) -> usize {
        let mut delta: isize = 0;
        for e in &self.edits {
            let ins = e.inserted_len() as isize;
            if pos < e.start || (pos == e.start && (e.start != e.end || assoc == Assoc::Before)) {
                break;
            }
            if pos >= e.end {
                delta += ins - e.deleted_len() as isize;
                continue;
            }
            // Inside a deleted range: collapse to the replacement's edge.
            let base = e.start as isize + delta;
            return (base + if assoc == Assoc::After { ins } else { 0 }) as usize;
        }
        (pos as isize + delta) as usize
    }

    /// Net change in length (chars).
    pub fn len_delta(&self) -> isize {
        self.edits
            .iter()
            .map(|e| e.inserted_len() as isize - e.deleted_len() as isize)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn apply(text: &str, cs: &ChangeSet) -> String {
        let mut r = Rope::from_str(text);
        cs.apply(&mut r);
        r.to_string()
    }

    #[test]
    fn multiple_edits_apply_in_pre_change_coordinates() {
        let cs = ChangeSet::new(vec![Edit::new(0, 1, "H"), Edit::new(6, 11, "there")]).unwrap();
        assert_eq!(apply("hello world", &cs), "Hello there");
    }

    #[test]
    fn overlapping_edits_are_rejected() {
        let err = ChangeSet::new(vec![Edit::new(0, 5, ""), Edit::new(3, 6, "")]);
        assert_eq!(err, Err(ChangeError::Overlap(3)));
    }

    #[test]
    fn inserts_at_same_position_keep_order() {
        let cs = ChangeSet::new(vec![Edit::new(1, 1, "A"), Edit::new(1, 1, "B")]).unwrap();
        assert_eq!(apply("xy", &cs), "xABy");
    }

    #[test]
    fn map_pos_shifts_after_inserts() {
        let cs = ChangeSet::insert(2, "abc");
        assert_eq!(cs.map_pos(1, Assoc::After), 1);
        assert_eq!(cs.map_pos(2, Assoc::Before), 2);
        assert_eq!(cs.map_pos(2, Assoc::After), 5);
        assert_eq!(cs.map_pos(4, Assoc::After), 7);
    }

    #[test]
    fn map_pos_collapses_deleted_ranges() {
        let cs = ChangeSet::replace(2, 6, "x");
        assert_eq!(cs.map_pos(4, Assoc::Before), 2);
        assert_eq!(cs.map_pos(4, Assoc::After), 3);
        assert_eq!(cs.map_pos(6, Assoc::After), 3);
        assert_eq!(cs.map_pos(8, Assoc::After), 5);
    }

    fn arb_change(len: usize) -> impl Strategy<Value = ChangeSet> {
        prop::collection::vec((0..=len, 0..=4usize, "[a-zé👍\n]{0,4}"), 0..4).prop_map(move |raw| {
            let mut edits: Vec<Edit> = Vec::new();
            let mut cursor = 0;
            let mut sorted = raw;
            sorted.sort_by_key(|r| r.0);
            for (start, del, text) in sorted {
                let start = start.max(cursor).min(len);
                let end = (start + del).min(len);
                edits.push(Edit::new(start, end, text));
                cursor = end;
            }
            ChangeSet::new(edits).unwrap()
        })
    }

    fn text_and_change() -> impl Strategy<Value = (String, ChangeSet)> {
        "[a-z é👍\n]{0,40}".prop_flat_map(|text| {
            let len = text.chars().count();
            (Just(text), arb_change(len))
        })
    }

    proptest! {
        #[test]
        fn invert_restores_original((text, cs) in text_and_change()) {
            let before = Rope::from_str(&text);
            let mut after = before.clone();
            cs.apply(&mut after);
            prop_assert_eq!(after.len_chars() as isize, text.chars().count() as isize + cs.len_delta());
            cs.invert(&before).apply(&mut after);
            prop_assert_eq!(after.to_string(), text);
        }
    }
}
