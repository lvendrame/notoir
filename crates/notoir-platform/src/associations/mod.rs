//! Opt-in default handlers. All entry points run on a worker; callers inject a
//! backend in tests. The journal is committed before changing a default.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, sync::Mutex};

mod native;
pub use native::NativeBackend;

pub const EXTENSIONS: &[&str] = &[
    "bash", "c", "cfg", "cjs", "conf", "cpp", "cs", "css", "csv", "go", "h", "hpp", "htm", "html",
    "ini", "java", "js", "json", "jsonc", "jsonl", "jsx", "kt", "kts", "log", "lua", "markdown",
    "md", "mjs", "ndjson", "php", "ps1", "py", "pyw", "r", "rb", "rs", "scss", "sh", "sql",
    "svelte", "swift", "text", "toml", "ts", "tsa", "tsv", "tsx", "txt", "vue", "xml", "yaml",
    "yml", "zsh",
];
static OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Association {
    pub extension: String,
    pub content_type: String,
    pub handler: Option<String>,
    pub is_notoir: bool,
}

/// Native errors are diagnostic details displayed below a localized message.
pub trait Backend: Send + Sync {
    fn inspect(&self) -> Result<Vec<Association>, String>;
    fn register(&self) -> Result<(), String>;
    /// `None` removes our override or selects the first usable OS fallback.
    fn change(&self, content_type: &str, handler: Option<&str>) -> Result<(), String>;
    fn identity(&self) -> &str;
    fn platform(&self) -> &str;
    fn confirm(&self) -> Result<(), String> {
        Ok(())
    }
    fn needs_confirmation(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
pub struct Selection {
    pub content_type: String,
    pub expected_handler: Option<String>,
    pub selected: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub rows: Vec<Association>,
    pub errors: BTreeMap<String, String>,
    pub pending: bool,
}
#[derive(Default, Deserialize, Serialize)]
struct Journal {
    version: u32,
    previous: BTreeMap<String, Option<String>>,
}
fn read_journal(path: &Path) -> Result<Journal, String> {
    match fs::read(path) {
        Ok(bytes) => {
            let journal: Journal = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if journal.version != 1 {
                return Err("Unsupported association journal version".into());
            }
            Ok(journal)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Journal {
            version: 1,
            ..Default::default()
        }),
        Err(e) => Err(e.to_string()),
    }
}
fn write_journal(path: &Path, journal: &Journal) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing journal directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(file.as_file_mut(), journal).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn inspect(backend: &dyn Backend) -> Result<Vec<Association>, String> {
    let _lock = OPERATIONS.lock().map_err(|e| e.to_string())?;
    let mut rows = backend.inspect()?;
    rows.sort_by(|a, b| a.extension.cmp(&b.extension));
    Ok(rows)
}
pub fn apply(
    backend: &dyn Backend,
    path: &Path,
    selections: &[Selection],
) -> Result<Outcome, String> {
    let _lock = OPERATIONS.lock().map_err(|e| e.to_string())?;
    let mut journal = read_journal(path)?;
    let before = backend.inspect()?;
    backend.register()?;
    let mut result = Outcome::default();
    let mut confirmation = false;
    for selection in selections {
        let Some(current) = before
            .iter()
            .find(|r| r.content_type == selection.content_type)
        else {
            result.errors.insert(
                selection.content_type.clone(),
                "File type is no longer available".into(),
            );
            continue;
        };
        if current.is_notoir == selection.selected {
            continue;
        }
        if current.handler != selection.expected_handler {
            result.errors.insert(
                selection.content_type.clone(),
                "Default application changed outside Notoir; refresh and review this type".into(),
            );
            continue;
        }
        let key = format!("{}:{}", backend.platform(), selection.content_type);
        let target = if selection.selected {
            journal
                .previous
                .insert(key.clone(), current.handler.clone());
            // Do not change anything if the prior default could not be saved.
            write_journal(path, &journal)?;
            Some(backend.identity().to_owned())
        } else {
            journal.previous.get(&key).cloned().flatten()
        };
        match backend.change(&selection.content_type, target.as_deref()) {
            Ok(()) => confirmation |= backend.needs_confirmation(),
            Err(error) => {
                result.errors.insert(selection.content_type.clone(), error);
            }
        }
    }
    if confirmation {
        backend.confirm()?;
    }
    result.rows = backend.inspect()?;
    result.rows.sort_by(|a, b| a.extension.cmp(&b.extension));
    for selection in selections {
        if result.errors.contains_key(&selection.content_type) {
            continue;
        }
        let verified = result.rows.iter().any(|r| {
            r.content_type == selection.content_type
                && r.is_notoir == selection.selected
                && (selection.selected || r.handler.is_some())
        });
        if !verified {
            result.pending = true;
            if !backend.needs_confirmation() {
                result.errors.insert(
                    selection.content_type.clone(),
                    "The operating system did not apply the requested default".into(),
                );
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Fake {
        rows: Mutex<Vec<Association>>,
        fail: Mutex<bool>,
        pending: bool,
        calls: Mutex<Vec<(String, Option<String>)>>,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                rows: Mutex::new(vec![
                    Association {
                        extension: "txt".into(),
                        content_type: "text/plain".into(),
                        handler: Some("old.app".into()),
                        is_notoir: false,
                    },
                    Association {
                        extension: "log".into(),
                        content_type: "text/plain".into(),
                        handler: Some("old.app".into()),
                        is_notoir: false,
                    },
                ]),
                fail: Mutex::new(false),
                pending: false,
                calls: Mutex::new(vec![]),
            }
        }
    }
    impl Backend for Fake {
        fn inspect(&self) -> Result<Vec<Association>, String> {
            Ok(self.rows.lock().unwrap().clone())
        }
        fn register(&self) -> Result<(), String> {
            Ok(())
        }
        fn change(&self, content_type: &str, handler: Option<&str>) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push((content_type.into(), handler.map(str::to_owned)));
            if *self.fail.lock().unwrap() {
                return Err("Denied".into());
            }
            if !self.pending {
                for row in self
                    .rows
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .filter(|r| r.content_type == content_type)
                {
                    row.handler = Some(handler.unwrap_or("fallback.app").into());
                    row.is_notoir = handler == Some(self.identity());
                }
            }
            Ok(())
        }
        fn identity(&self) -> &str {
            "notoir.app"
        }
        fn platform(&self) -> &str {
            "test"
        }
        fn needs_confirmation(&self) -> bool {
            self.pending
        }
    }
    fn selection(selected: bool, expected: &str) -> Selection {
        Selection {
            content_type: "text/plain".into(),
            expected_handler: Some(expected.into()),
            selected,
        }
    }
    #[test]
    fn journals_then_restores_previous_default_across_service_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let backend = Fake::new();
        let outcome = apply(&backend, &path, &[selection(true, "old.app")]).unwrap();
        assert!(outcome.errors.is_empty());
        assert!(outcome.rows.iter().all(|r| r.is_notoir));
        assert_eq!(
            read_journal(&path).unwrap().previous["test:text/plain"],
            Some("old.app".into())
        );
        let outcome = apply(&backend, &path, &[selection(false, "notoir.app")]).unwrap();
        assert!(outcome.errors.is_empty());
        assert!(
            outcome
                .rows
                .iter()
                .all(|r| r.handler.as_deref() == Some("old.app"))
        );
    }
    #[test]
    fn external_change_and_unwritable_journal_prevent_default_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Fake::new();
        let outcome = apply(
            &backend,
            &dir.path().join("journal"),
            &[selection(true, "other.app")],
        )
        .unwrap();
        assert_eq!(outcome.errors.len(), 1);
        assert!(backend.calls.lock().unwrap().is_empty());
        let blocking = dir.path().join("file");
        fs::write(&blocking, "not a directory").unwrap();
        assert!(
            apply(
                &backend,
                &blocking.join("journal"),
                &[selection(true, "old.app")]
            )
            .is_err()
        );
        assert!(backend.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn pending_confirmation_and_failure_are_not_reported_as_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = Fake::new();
        backend.pending = true;
        let outcome = apply(
            &backend,
            &dir.path().join("journal"),
            &[selection(true, "old.app")],
        )
        .unwrap();
        assert!(outcome.pending);
        assert!(outcome.rows.iter().all(|r| !r.is_notoir));
        backend.pending = false;
        *backend.fail.lock().unwrap() = true;
        let outcome = apply(
            &backend,
            &dir.path().join("journal"),
            &[selection(true, "old.app")],
        )
        .unwrap();
        assert_eq!(outcome.errors["text/plain"], "Denied");
    }
    #[test]
    fn preexisting_notoir_default_without_history_uses_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Fake::new();
        backend.change("text/plain", Some("notoir.app")).unwrap();
        let outcome = apply(
            &backend,
            &dir.path().join("journal"),
            &[selection(false, "notoir.app")],
        )
        .unwrap();
        assert!(
            outcome
                .rows
                .iter()
                .all(|r| r.handler.as_deref() == Some("fallback.app"))
        );
    }
}
