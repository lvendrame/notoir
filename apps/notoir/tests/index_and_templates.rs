//! Indexed full-text search and templates (spec 34): the disposable search
//! index in the cache folder, its fallback to streaming search, and New File
//! from Template… with and without a workspace.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use serde_json::json;

fn wait_for(app: &mut common::TestApp, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    (d, root)
}

fn open(root: &Path) -> common::TestApp {
    let paths = vec![(root.to_path_buf(), None)];
    let mut app = common::start(move |o| o.open_paths = paths);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .workspace
        .is_some()));
    app
}

fn index_ready(app: &common::TestApp) -> bool {
    app.harness.state().core.text_index_dir().is_some()
}

/// Runs a workspace search to the end: (files with matches, indexed).
fn search(app: &mut common::TestApp, text: &str) -> (Vec<String>, bool) {
    {
        let core = &mut app.harness.state_mut().core;
        core.search.query = text.into();
        core.start_search();
    }
    assert!(wait_for(app, |a| a.harness.state().core.search.done));
    let st = &app.harness.state().core.search;
    let mut rels: Vec<String> = st.results.iter().map(|f| f.rel.clone()).collect();
    rels.sort();
    (rels, st.indexed)
}

const FILES: &[(&str, &str)] = &[
    ("README.md", "Notoir keeps a needle here.\n"),
    ("docs/guide.md", "Another NEEDLE in the guide.\n"),
    ("docs/other.md", "nothing to see\n"),
];

// spec 34: "Index is disposable"
#[test]
fn index_is_disposable() {
    let (_d, root) = workspace(FILES);
    // Enough files that rebuilding visibly outlasts startup.
    for i in 0..2000 {
        let p = root.join(format!("filler/{}/f{i}.txt", i % 20));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, format!("filler text number {i}\n").repeat(20)).unwrap();
    }
    let mut app = open(&root);
    assert!(wait_for(&mut app, index_ready), "index built");
    let expected = vec!["README.md".to_owned(), "docs/guide.md".to_owned()];
    assert_eq!(search(&mut app, "needle"), (expected.clone(), true));
    let (cache, dir) = {
        let core = &app.harness.state().core;
        let dir = core.text_index_dir().unwrap();
        (core.paths.cache.clone(), dir)
    };
    assert!(
        dir.starts_with(cache.join("index")),
        "index lives in the cache"
    );
    assert!(dir.join("text.sqlite").exists());
    // Nothing is written into the workspace.
    assert!(!root.join(".notoir").exists());

    // The index is deleted while Notoir is closed.
    let between_dir = dir.clone();
    let mut app = common::restart_with(
        app,
        move || std::fs::remove_dir_all(&between_dir).unwrap(),
        move |o| o.open_paths = vec![(root.clone(), None)],
    );
    // Search works at once, by streaming, while the index rebuilds.
    assert!(!index_ready(&app), "index not usable yet");
    assert_eq!(search(&mut app, "needle"), (expected.clone(), false));
    assert!(wait_for(&mut app, index_ready), "index rebuilt");
    assert!(dir.join("text.sqlite").exists());
    assert_eq!(search(&mut app, "needle"), (expected, true));
}

#[test]
fn index_follows_changes_and_rebuilds_on_request() {
    let (_d, root) = workspace(FILES);
    let mut app = open(&root);
    assert!(wait_for(&mut app, index_ready));
    // Edited and new files are searched after the watcher reports them.
    std::fs::write(root.join("docs/other.md"), "now with a zebra\n").unwrap();
    std::fs::write(root.join("new.md"), "zebra too\n").unwrap();
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        index_ready(a)
            && core
                .workspace
                .as_ref()
                .is_some_and(|w| w.index.iter().any(|e| e.rel == "new.md"))
    }));
    assert_eq!(
        search(&mut app, "zebra"),
        (vec!["docs/other.md".into(), "new.md".into()], true)
    );
    // Short and regex queries stream.
    assert!(!search(&mut app, "ze").1);
    app.harness.state_mut().core.search.regex = true;
    assert!(!search(&mut app, "zeb.a").1);
    app.harness.state_mut().core.search.regex = false;

    // Rebuild: streaming until it finishes, then indexed again.
    common::run(&mut app, "search.rebuildIndex", json!({}));
    assert!(wait_for(&mut app, index_ready));
    assert!(search(&mut app, "zebra").1);
    assert_eq!(
        app.harness.state().core.text_index.last.map(|s| s.added),
        Some(4),
        "every file indexed again"
    );

    // Turning the index off streams.
    {
        let core = &mut app.harness.state_mut().core;
        core.set_user_setting(
            "search.useIndex",
            Some(notoir_core::settings::SettingValue::Bool(false)),
        );
    }
    app.harness.run_steps(2);
    assert!(!index_ready(&app));
    assert!(!search(&mut app, "zebra").1);
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[test]
fn new_file_from_a_workspace_template() {
    let (_d, root) = workspace(&[
        (
            ".notoir/templates/Meeting.md",
            "# {{title}}\n\nDate: {{date}}\n\n- {{cursor}}\n",
        ),
        ("notes/a.md", "a\n"),
    ]);
    let mut app = open(&root);
    common::run(&mut app, "file.newFromTemplate", json!({}));
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_all_by_label_contains("Meeting")
        .next()
        .is_some()));
    let tpl = root.join(".notoir/templates/Meeting.md");
    common::run(
        &mut app,
        "file.pickTemplate",
        json!({ "path": tpl.to_string_lossy() }),
    );
    // The new file is named inline in the Explorer.
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .workspace
        .as_ref()
        .is_some_and(|w| w.edit.is_some())));
    {
        let core = &mut app.harness.state_mut().core;
        core.workspace.as_mut().unwrap().edit.as_mut().unwrap().text = "Standup".into();
        core.commit_edit();
    }
    let file = root.join("Standup.md");
    let text = format!("# Standup\n\nDate: {}\n\n- \n", today());
    // A new file takes the platform's line ending (the buffer keeps `\n`).
    let expected = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.clone()
    };
    // Created, filled and saved through the pipeline.
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        std::fs::read_to_string(&file).is_ok_and(|t| t == expected)
            && core
                .workbench
                .active_document()
                .and_then(|d| core.workbench.document(d))
                .is_some_and(|d| !d.is_dirty())
    }));
    let core = &app.harness.state().core;
    let view = core.workbench.active_view().unwrap();
    let caret = core.workbench.view(view).unwrap().selection.primary().head;
    assert_eq!(caret, text.find("- ").unwrap() + 2, "caret at {{cursor}}");
}

#[test]
fn templates_without_a_workspace_and_none_found() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newFromTemplate", json!({}));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .notifications
        .history()
        .any(|n| n.message.key == "notifications:noTemplates")));

    let personal = app.harness.state().core.paths.config.join("templates");
    std::fs::create_dir_all(&personal).unwrap();
    std::fs::write(personal.join("Letter.txt"), "Dear {{cursor}},\n{{date}}\n").unwrap();
    common::run(&mut app, "file.newFromTemplate", json!({}));
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .templates
        .list
        .is_empty()));
    common::run(
        &mut app,
        "file.pickTemplate",
        json!({ "path": personal.join("Letter.txt").to_string_lossy() }),
    );
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        core.workbench
            .active_document()
            .and_then(|d| core.workbench.document(d))
            .is_some_and(|d| d.buffer.text() == format!("Dear ,\n{}\n", today()))
    }));
    let core = &app.harness.state().core;
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert!(doc.is_dirty() && doc.path().is_none(), "untitled, unsaved");
}
