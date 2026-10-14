//! Typed application events (docs/specs/06-event-system.md).
//!
//! Domain services emit into an [`EventOutbox`] while handling commands; the
//! app drains it once per frame to a fixed list of subscribers, which may only
//! enqueue commands or start jobs — never mutate state re-entrantly.

use std::sync::Arc;

use notoir_editor::ChangeSet;

use crate::ids::{DocumentId, GroupId, ViewId};

/// Who changed a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeOrigin {
    User { view: ViewId },
    Undo,
    Redo,
    Reload,
    Format,
    Internal,
}

#[derive(Clone, Debug)]
pub enum AppEvent {
    ApplicationStarted,
    ApplicationExiting,
    DocumentOpened {
        doc: DocumentId,
    },
    DocumentChanged {
        doc: DocumentId,
        revision: u64,
        origin: ChangeOrigin,
        changes: Arc<ChangeSet>,
    },
    DocumentSaved {
        doc: DocumentId,
    },
    DocumentClosed {
        doc: DocumentId,
    },
    ActiveDocumentChanged {
        doc: Option<DocumentId>,
        view: Option<ViewId>,
    },
    EditorViewCreated {
        view: ViewId,
        doc: DocumentId,
        group: GroupId,
    },
    EditorViewClosed {
        view: ViewId,
    },
    LayoutChanged,
    SettingsChanged {
        keys: Vec<String>,
    },
    ThemeChanged {
        theme: String,
    },
    LanguageChanged {
        lang: String,
    },
}

#[derive(Debug, Default)]
pub struct EventOutbox {
    events: Vec<AppEvent>,
}

impl EventOutbox {
    pub fn emit(&mut self, event: AppEvent) {
        self.events.push(event);
    }

    /// Takes all pending events in emission order.
    pub fn drain(&mut self) -> Vec<AppEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn pending(&self) -> &[AppEvent] {
        &self.events
    }
}
