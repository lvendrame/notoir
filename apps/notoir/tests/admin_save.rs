//! Saving a file that needs administrator authorization (spec 36). The
//! elevation flow is scripted; the target is made unwritable with
//! permissions.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable;
use notoir_platform::elevation::{Elevation, ElevationError};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Authorize,
    Cancel,
    FailAfterAuthorizing,
}

struct Scripted {
    answer: Answer,
    calls: Mutex<Vec<PathBuf>>,
}

impl Elevation for Scripted {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<(), ElevationError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        match self.answer {
            Answer::Cancel => Err(ElevationError::Cancelled),
            Answer::FailAfterAuthorizing => Err(ElevationError::Failed("disk error".into())),
            Answer::Authorize => {
                // "Administrator": lift the protection for the write only.
                let dir = path.parent().unwrap();
                set_mode(dir, 0o755);
                set_mode(path, 0o644);
                std::fs::write(path, bytes).map_err(|e| ElevationError::Failed(e.to_string()))?;
                set_mode(path, 0o444);
                set_mode(dir, 0o555);
                Ok(())
            }
        }
    }
}

fn set_mode(p: &Path, mode: u32) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A protected file (read-only, in a read-only folder) open and edited.
struct Fixture {
    _dir: tempfile::TempDir,
    folder: PathBuf,
    file: PathBuf,
    app: common::TestApp,
    elevation: Arc<Scripted>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_mode(&self.folder, 0o755);
        let _ = std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o644));
    }
}

fn protected(answer: Answer) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("etc");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("hosts.conf");
    std::fs::write(&file, "127.0.0.1 localhost\n").unwrap();
    let file = file.canonicalize().unwrap();
    let folder = folder.canonicalize().unwrap();
    set_mode(&file, 0o444);
    set_mode(&folder, 0o555);
    let elevation = Arc::new(Scripted {
        answer,
        calls: Mutex::new(Vec::new()),
    });
    let el = elevation.clone();
    let path = file.clone();
    let mut app = common::start(move |o| {
        o.open_paths = vec![(path, None)];
        o.elevation = Some(el);
    });
    common::settle(&mut app);
    // It opens read-only (file system); the user unlocks it to edit.
    {
        let core = &app.harness.state().core;
        let doc = core.workbench.active_document().unwrap();
        assert_eq!(
            core.workbench.document(doc).unwrap().read_only,
            notoir_core::document::ReadOnly::FileSystem
        );
    }
    common::run(&mut app, "edit.toggleReadOnly", Value::Null);
    {
        let core = &mut app.harness.state_mut().core;
        let doc = core.workbench.active_document().unwrap();
        let len = core.workbench.document(doc).unwrap().buffer.len_chars();
        let view = core.workbench.active_view().unwrap();
        core.workbench
            .edit(
                view,
                notoir_editor::ChangeSet::insert(len, "10.0.0.1 router\n"),
                notoir_editor::Selection::caret(len),
                notoir_editor::EditKind::Other,
                0,
            )
            .unwrap();
    }
    common::run(&mut app, "recovery.flush", Value::Null);
    common::settle(&mut app);
    Fixture {
        _dir: dir,
        folder,
        file,
        app,
        elevation,
    }
}

fn doc(app: &common::TestApp) -> notoir_core::ids::DocumentId {
    app.harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap()
}

fn dirty(app: &common::TestApp) -> bool {
    let core = &app.harness.state().core;
    core.workbench.document(doc(app)).unwrap().is_dirty()
}

/// Save fails with "permission denied"; the toast offers the admin save.
fn save_and_choose_admin(f: &mut Fixture) {
    common::run(&mut f.app, "file.save", Value::Null);
    common::settle(&mut f.app);
    assert!(dirty(&f.app), "plain save failed");
    f.app
        .harness
        .get_by_label_contains("Save as Administrator…")
        .click();
    f.app.harness.run_steps(3);
    common::settle(&mut f.app);
}

fn has_recovery(app: &common::TestApp) -> bool {
    let core = &app.harness.state().core;
    core.recovery.entries.contains_key(&doc(app))
}

// spec 36: "Administrator authorization saves the protected file"
#[test]
fn authorized_admin_save_writes_and_cleans() {
    let mut f = protected(Answer::Authorize);
    save_and_choose_admin(&mut f);
    assert_eq!(
        *f.elevation.calls.lock().unwrap(),
        vec![f.file.clone()],
        "retried to the same path"
    );
    assert_eq!(
        std::fs::read_to_string(&f.file).unwrap(),
        "127.0.0.1 localhost\n10.0.0.1 router\n"
    );
    assert!(!dirty(&f.app), "clean after the save");
    // The pipeline recorded the new disk hash: no conflict on the next save.
    let core = &f.app.harness.state().core;
    let d = core.workbench.document(doc(&f.app)).unwrap();
    assert!(d.saved_content_hash.is_some());
}

// spec 36: "Canceling administrator authorization preserves edits"
#[test]
fn cancelled_admin_save_keeps_edits() {
    let mut f = protected(Answer::Cancel);
    assert!(has_recovery(&f.app));
    save_and_choose_admin(&mut f);
    assert_eq!(f.elevation.calls.lock().unwrap().len(), 1);
    assert!(dirty(&f.app), "still dirty");
    assert!(has_recovery(&f.app), "recovery snapshot kept");
    assert_eq!(
        std::fs::read_to_string(&f.file).unwrap(),
        "127.0.0.1 localhost\n"
    );
    assert!(
        f.app.harness.query_by_label_contains("Save As…").is_some(),
        "Save As offered"
    );
    assert!(
        f.app
            .harness
            .query_by_label_contains("Save as Administrator…")
            .is_none(),
        "no loop of authorization prompts"
    );
}

// spec 36: "Administrator retry failure preserves edits"
#[test]
fn failed_admin_retry_keeps_edits_and_reports() {
    let mut f = protected(Answer::FailAfterAuthorizing);
    save_and_choose_admin(&mut f);
    assert!(dirty(&f.app));
    assert!(has_recovery(&f.app));
    assert!(
        f.app
            .harness
            .query_by_label_contains("Could not save hosts.conf as administrator")
            .is_some(),
        "failure shown"
    );
    assert!(f.app.harness.query_by_label_contains("Save As…").is_some());
}

#[test]
fn no_admin_offer_for_other_failures() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "x").unwrap();
    let f = f.canonicalize().unwrap();
    let path = f.clone();
    let mut app = common::start(move |o| o.open_paths = vec![(path, None)]);
    common::settle(&mut app);
    // The folder vanishes: "folder not found", not a permission problem.
    let core = &mut app.harness.state_mut().core;
    let d = core.workbench.active_document().unwrap();
    core.workbench.document_mut(d).unwrap().origin = notoir_core::document::DocumentOrigin::File {
        path: dir.path().join("missing/a.txt"),
        key: notoir_core::document::PathKey::new(&dir.path().join("missing/a.txt"), false),
    };
    let len = core.workbench.document(d).unwrap().buffer.len_chars();
    let view = core.workbench.active_view().unwrap();
    core.workbench
        .edit(
            view,
            notoir_editor::ChangeSet::insert(len, "y"),
            notoir_editor::Selection::caret(len),
            notoir_editor::EditKind::Other,
            0,
        )
        .unwrap();
    common::run(&mut app, "file.save", Value::Null);
    common::settle(&mut app);
    assert!(app.harness.query_by_label_contains("Save As…").is_some());
    assert!(
        app.harness
            .query_by_label_contains("Save as Administrator…")
            .is_none()
    );
}
