//! Help › Documentation and context help on F1 (spec 18).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use egui::Key;
use egui_kittest::kittest::Queryable;
use notoir::files::ScriptedDialogs;
use serde_json::Value;

const GUIDE: &str = "https://lvendrame.github.io/notoir/";

fn start(dialogs: &ScriptedDialogs, paths: Vec<PathBuf>) -> common::TestApp {
    let d = dialogs.clone();
    let mut app = common::start(move |o| {
        o.dialogs = Some(Arc::new(d));
        o.open_paths = paths.into_iter().map(|p| (p, None)).collect();
    });
    common::settle(&mut app);
    app
}

fn last_opened(dialogs: &ScriptedDialogs) -> Option<String> {
    dialogs.opened.lock().unwrap().last().cloned()
}

fn press_f1(app: &mut common::TestApp) {
    app.harness.key_press(Key::F1);
    app.harness.run_steps(3);
}

/// A canonical temp folder with `files` (name, text) in it.
fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    for (name, text) in files {
        std::fs::write(root.join(name), text).unwrap();
    }
    (d, root)
}

// spec 18: "Documentation opens the user guide"
#[test]
fn documentation_opens_the_user_guide() {
    let dialogs = ScriptedDialogs::default();
    let mut app = start(&dialogs, vec![]);
    common::run(&mut app, "help.documentation", Value::Null);
    assert_eq!(last_opened(&dialogs).as_deref(), Some(GUIDE));
    common::run(&mut app, "help.keyboardReference", Value::Null);
    assert_eq!(
        last_opened(&dialogs),
        Some(format!("{GUIDE}reference/shortcuts.html"))
    );
}

// spec 18: "F1 opens the guide page for the focused feature"
#[test]
fn f1_opens_the_page_for_the_focused_feature() {
    let (_d, root) = folder(&[("a.txt", "x"), ("b.md", "# B")]);
    let dialogs = ScriptedDialogs::default();
    let mut app = start(&dialogs, vec![root.clone()]);
    app.harness.get_by_label("a.txt").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    assert!(app.harness.state().core.explorer_focused);
    press_f1(&mut app);
    assert_eq!(
        last_opened(&dialogs),
        Some(format!("{GUIDE}workspaces.html"))
    );

    common::run(
        &mut app,
        "explorer.open",
        serde_json::json!({ "path": root.join("b.md").to_string_lossy() }),
    );
    common::settle(&mut app);
    app.harness.state_mut().core.explorer_focused = false;
    app.harness.state_mut().core.editor_focused = true;
    press_f1(&mut app);
    assert_eq!(last_opened(&dialogs), Some(format!("{GUIDE}markdown.html")));
}

#[test]
fn f1_follows_the_active_tab_and_editor() {
    let (_d, root) = folder(&[("a.txt", "x"), ("c.json", "{}")]);
    let dialogs = ScriptedDialogs::default();
    let mut app = start(&dialogs, vec![root.join("c.json")]);
    press_f1(&mut app);
    assert_eq!(last_opened(&dialogs), Some(format!("{GUIDE}json.html")));

    common::run(&mut app, "app.settings", Value::Null);
    press_f1(&mut app);
    assert_eq!(last_opened(&dialogs), Some(format!("{GUIDE}settings.html")));

    common::run(&mut app, "file.closeAll", Value::Null);
    common::settle(&mut app);
    common::run(
        &mut app,
        "explorer.open",
        serde_json::json!({ "path": root.join("a.txt").to_string_lossy() }),
    );
    common::settle(&mut app);
    app.harness.state_mut().core.editor_focused = true;
    press_f1(&mut app);
    assert_eq!(last_opened(&dialogs), Some(format!("{GUIDE}editing.html")));
}

#[test]
fn f1_without_context_opens_the_start_page() {
    let dialogs = ScriptedDialogs::default();
    let mut app = start(&dialogs, vec![]);
    app.harness.state_mut().core.panels.primary.visible = false;
    press_f1(&mut app);
    assert_eq!(last_opened(&dialogs).as_deref(), Some(GUIDE));
}

#[test]
fn f1_no_longer_opens_the_command_palette() {
    let dialogs = ScriptedDialogs::default();
    let mut app = start(&dialogs, vec![]);
    press_f1(&mut app);
    assert!(app.harness.state().ui.picker.is_none());
    assert!(last_opened(&dialogs).is_some_and(|u| u.starts_with(GUIDE)));
}
