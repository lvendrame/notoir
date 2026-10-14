//! Process-unique ids. Never reused within a session.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($($name:ident),*) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub u64);
    )*};
}

id_type!(DocumentId, ViewId, GroupId);

/// Monotonic id source shared by all id kinds of one window.
#[derive(Debug, Default)]
pub struct IdGen {
    next: u64,
}

impl IdGen {
    pub fn fresh(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}
