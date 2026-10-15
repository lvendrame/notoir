//! Syntax highlighting in the editor (spec 26).

mod common;

use std::path::{Path, PathBuf};

use notoir_editor::syntax::TokenKind;
use serde_json::json;

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn kinds(app: &common::TestApp, line: usize) -> Vec<TokenKind> {
    let v = app.harness.state().core.workbench.active_view().unwrap();
    app.harness
        .state()
        .ui
        .editors
        .spans(v, line)
        .unwrap_or_default()
        .iter()
        .map(|s| s.kind)
        .collect()
}

#[test]
fn toml_and_typescript_show_syntax_colours() {
    let dir = tempfile::tempdir().unwrap();
    let toml = write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"notoir\"\nversion = 1\n",
    );
    let ts = write(dir.path(), "app.ts", "const answer: number = 42; // ok\n");
    let mut app = common::start(move |o| o.open_paths = vec![(toml, None), (ts, None)]);
    common::settle(&mut app);
    let ts_kinds = kinds(&app, 0);
    assert!(
        ts_kinds.contains(&TokenKind::Keyword)
            && ts_kinds.contains(&TokenKind::Number)
            && ts_kinds.contains(&TokenKind::Comment),
        "{ts_kinds:?}"
    );
    let first = app.harness.state().core.workbench.layout.active().tabs[0];
    common::run(&mut app, "view.activate", json!({ "view": first.0 }));
    assert!(
        kinds(&app, 1).contains(&TokenKind::String),
        "{:?}",
        kinds(&app, 1)
    );
}

#[test]
fn language_override_switches_grammar_and_plain_text_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "notes.txt", "# Title\n\nSome *text*.\n");
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    assert!(kinds(&app, 0).is_empty(), "plain text");
    common::run(
        &mut app,
        "editor.changeLanguageMode",
        json!({ "language": "markdown" }),
    );
    app.harness.run_steps(2);
    assert!(
        kinds(&app, 0).contains(&TokenKind::Heading),
        "{:?}",
        kinds(&app, 0)
    );
}

#[test]
fn typing_rehighlights_following_lines() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(
        dir.path(),
        "main.rs",
        "let a = 1;\nlet b = 2;\nlet c = 3;\n",
    );
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    assert!(kinds(&app, 2).contains(&TokenKind::Keyword));
    // Open a block comment at the start: the rest becomes a comment.
    app.harness.event(egui::Event::Text("/*".into()));
    app.harness.run_steps(2);
    assert_eq!(kinds(&app, 2), vec![TokenKind::Comment]);
}

#[test]
fn large_files_are_not_highlighted() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "main.rs", "let a = 1;\n");
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    {
        let core = &mut app.harness.state_mut().core;
        let d = core.workbench.active_document().unwrap();
        core.workbench.document_mut(d).unwrap().large_file = true;
    }
    app.harness.run_steps(2);
    assert!(kinds(&app, 0).is_empty());
}
