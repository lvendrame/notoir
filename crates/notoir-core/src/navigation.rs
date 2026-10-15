//! Navigation history (docs/specs/15 §Navigation History): locations visited
//! by switching documents or by command jumps, walked with Back / Forward.

use std::path::PathBuf;

use crate::ids::DocumentId;

/// Where a location lives: a file survives closing its tab; other documents
/// only while open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocRef {
    Path(PathBuf),
    Scratchpad(String),
    Open(DocumentId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub doc: DocRef,
    /// 0-based line and char column.
    pub line: usize,
    pub col: usize,
}

pub const MAX_ENTRIES: usize = 50;
/// Locations this close in the same document merge into one entry.
pub const MERGE_LINES: usize = 3;

#[derive(Debug, Default)]
pub struct NavigationHistory {
    entries: Vec<Location>,
    /// The entry the user is at.
    index: usize,
}

impl NavigationHistory {
    fn near(a: &Location, b: &Location) -> bool {
        a.doc == b.doc && a.line.abs_diff(b.line) <= MERGE_LINES
    }

    /// A jump from `from` to `to`: forward entries are dropped.
    pub fn record(&mut self, from: Location, to: Location) {
        if self.entries.is_empty() {
            self.entries.push(from);
            self.index = 0;
        } else {
            self.entries.truncate(self.index + 1);
            // The current entry follows the cursor until the jump.
            self.entries[self.index] = from;
        }
        if !Self::near(&self.entries[self.index], &to) {
            self.entries.push(to);
        } else {
            self.entries[self.index] = to;
        }
        if self.entries.len() > MAX_ENTRIES {
            let drop = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..drop);
        }
        self.index = self.entries.len() - 1;
    }

    /// Back from `current`: the previous location, skipping ones `valid`
    /// rejects (deleted files).
    pub fn back(
        &mut self,
        current: Location,
        valid: impl Fn(&Location) -> bool,
    ) -> Option<Location> {
        self.step(current, valid, false)
    }

    pub fn forward(
        &mut self,
        current: Location,
        valid: impl Fn(&Location) -> bool,
    ) -> Option<Location> {
        self.step(current, valid, true)
    }

    fn step(
        &mut self,
        current: Location,
        valid: impl Fn(&Location) -> bool,
        fwd: bool,
    ) -> Option<Location> {
        if self.entries.is_empty() {
            return None;
        }
        // Remember where the user is so the opposite direction returns here.
        if Self::near(&self.entries[self.index], &current) || !fwd {
            self.entries[self.index] = current;
        }
        let mut i = self.index;
        loop {
            if fwd {
                if i + 1 >= self.entries.len() {
                    return None;
                }
                i += 1;
            } else {
                if i == 0 {
                    return None;
                }
                i -= 1;
            }
            if valid(&self.entries[i]) {
                self.index = i;
                return Some(self.entries[i].clone());
            }
        }
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(f: &str, line: usize) -> Location {
        Location {
            doc: DocRef::Path(PathBuf::from(f)),
            line,
            col: 0,
        }
    }

    // spec 15: "Back and forward across files"
    #[test]
    fn back_and_forward_across_files() {
        let mut h = NavigationHistory::default();
        h.record(at("start", 0), at("README.md", 91));
        h.record(at("README.md", 91), at("config.json", 43));
        h.record(at("config.json", 43), at("architecture.md", 9));
        let b1 = h.back(at("architecture.md", 9), |_| true).unwrap();
        assert_eq!(b1, at("config.json", 43));
        let b2 = h.back(b1, |_| true).unwrap();
        assert_eq!(b2, at("README.md", 91));
        let f = h.forward(b2, |_| true).unwrap();
        assert_eq!(f, at("config.json", 43));
        assert!(h.can_forward());
        // A new jump drops the forward entries.
        h.record(at("config.json", 43), at("other.md", 0));
        assert!(!h.can_forward());
    }

    #[test]
    fn merges_skips_and_caps() {
        let mut h = NavigationHistory::default();
        h.record(at("a", 0), at("a", 2));
        assert_eq!(h.len(), 1, "nearby jump merges");
        h.record(at("a", 2), at("gone", 50));
        h.record(at("gone", 50), at("b", 0));
        let back = h
            .back(at("b", 0), |l| l.doc != DocRef::Path("gone".into()))
            .unwrap();
        assert_eq!(back, at("a", 2), "deleted file skipped");
        for i in 0..100 {
            h.record(at("x", i * 100), at("x", i * 100 + 50));
        }
        assert!(h.len() <= MAX_ENTRIES);
    }
}
