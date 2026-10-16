//! Workspace search, Quick Open and navigation history (spec 15).

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Event, Key};
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

fn write(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}

fn folder() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    (d, root)
}

fn open_ws(root: &Path, files: Vec<PathBuf>) -> common::TestApp {
    let mut paths = vec![(root.to_path_buf(), None)];
    paths.extend(files.into_iter().map(|f| (f, None)));
    let mut app = common::start(move |o| o.open_paths = paths);
    common::settle(&mut app);
    app
}

fn wait_for(app: &mut common::TestApp, secs: u64, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn search(app: &mut common::TestApp, q: &str) {
    common::run(app, "search.workspace", Value::Null);
    let core = &mut app.harness.state_mut().core;
    core.search.query = q.into();
    core.start_search();
    assert!(
        wait_for(app, 10, |a| a.harness.state().core.search.done),
        "search finished"
    );
}

fn result_files(app: &common::TestApp) -> Vec<String> {
    app.harness
        .state()
        .core
        .search
        .results
        .iter()
        .map(|f| f.rel.clone())
        .collect()
}

fn active_path(app: &common::TestApp) -> Option<PathBuf> {
    let core = &app.harness.state().core;
    core.workbench
        .document(core.workbench.active_document()?)?
        .path()
        .map(Path::to_path_buf)
}

fn caret(app: &common::TestApp) -> (usize, usize, usize) {
    let core = &app.harness.state().core;
    let v = core
        .workbench
        .view(core.workbench.active_view().unwrap())
        .unwrap();
    let rope = core
        .workbench
        .document(v.document_id)
        .unwrap()
        .buffer
        .rope();
    let r = v.selection.primary();
    let line = rope.char_to_line(r.head);
    (
        line,
        r.from() - rope.line_to_char(rope.char_to_line(r.from())),
        r.to() - r.from(),
    )
}

fn lines(n: usize, special: &[(usize, &str)]) -> String {
    (1..=n)
        .map(|i| {
            special
                .iter()
                .find(|(l, _)| *l == i)
                .map_or(format!("line {i}\n"), |(_, t)| format!("{t}\n"))
        })
        .collect()
}

// spec 15: "Results stream without blocking typing"
#[test]
fn results_stream_without_blocking_typing() {
    let (_d, root) = folder();
    for i in 0..15_000 {
        write(
            &root,
            &format!("d{}/f{i}.txt", i % 50),
            if i % 7 == 0 {
                "an MCP line\n"
            } else {
                "nothing\n"
            },
        );
    }
    let note = write(&root, "note.txt", "x\n");
    let mut app = open_ws(&root, vec![note]);
    common::run(&mut app, "search.workspace", Value::Null);
    {
        let core = &mut app.harness.state_mut().core;
        core.search.query = "MCP".into();
        core.start_search();
    }
    // Type in the editor while the search runs.
    let v = app.harness.state().core.workbench.active_view().unwrap();
    app.harness.state_mut().ui.focus_editor = Some(v);
    app.harness.run_steps(2);
    let mut partial = false;
    let mut typed = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !app.harness.state().core.search.done && Instant::now() < deadline {
        app.harness.event(Event::Text("a".into()));
        typed += 1;
        app.harness.run_steps(1);
        let st = &app.harness.state().core.search;
        partial |= !st.done && !st.results.is_empty();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.harness.state().core.search.done);
    assert!(partial, "results appeared before the search finished");
    let core = &app.harness.state().core;
    let text = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .text();
    assert_eq!(
        text.matches('a').count(),
        typed,
        "every keystroke applied during the search"
    );
    assert_eq!(core.search.total(), 15_000_usize.div_ceil(7));
}

// spec 15: "Dirty buffers are searched in memory"
#[test]
fn dirty_buffers_searched_in_memory() {
    let (_d, root) = folder();
    let notes = write(&root, "notes.md", "hello\n");
    let mut app = open_ws(&root, vec![notes]);
    app.harness.state_mut().core.editor_focused = true;
    app.harness.event(Event::Text("zebra crossing ".into()));
    common::settle(&mut app);
    search(&mut app, "zebra");
    assert_eq!(result_files(&app), ["notes.md"]);
    let m = &app.harness.state().core.search.results[0].matches[0];
    assert_eq!(m.line, 0);
    assert!(m.preview.contains("zebra crossing hello"));
}

// spec 15: "Gitignored files are excluded"
#[test]
fn gitignored_files_excluded() {
    let (_d, root) = folder();
    write(&root, ".gitignore", "build/\n");
    write(&root, "build/out.txt", "MCP\n");
    write(&root, "docs/mcp.md", "MCP\n");
    let mut app = open_ws(&root, vec![]);
    search(&mut app, "MCP");
    assert_eq!(result_files(&app), ["docs/mcp.md"]);
}

// spec 15: "Clicking a result jumps to the match"
#[test]
fn clicking_result_jumps() {
    let (_d, root) = folder();
    write(
        &root,
        "docs/mcp.md",
        &lines(
            60,
            &[
                (14, "Notoir exposes an MCP server"),
                (48, "configure the MCP server"),
            ],
        ),
    );
    let mut app = open_ws(&root, vec![]);
    search(&mut app, "MCP");
    app.harness.get_by_label("docs/mcp.md:48").click();
    assert!(wait_for(&mut app, 20, |a| active_path(a)
        .is_some_and(|p| p.ends_with("docs/mcp.md"))));
    assert_eq!(caret(&app), (47, 14, 3), "match on line 48 selected");
}

#[test]
fn replace_all_across_open_and_closed_files() {
    let (_d, root) = folder();
    let open = write(&root, "a.md", "MCP one\n");
    write(&root, "b.md", "MCP two MCP\n");
    let mut app = open_ws(&root, vec![open]);
    search(&mut app, "MCP");
    app.harness.state_mut().core.search.replace = "Model".into();
    common::run(&mut app, "search.replaceAll", Value::Null);
    app.harness.run_steps(2);
    app.harness.get_by_label("Replace").click();
    assert!(wait_for(&mut app, 5, |a| a
        .harness
        .state()
        .core
        .search
        .done
        && a.harness.state().core.search.results.is_empty()));
    assert_eq!(
        std::fs::read_to_string(root.join("b.md")).unwrap(),
        "Model two Model\n"
    );
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    assert_eq!(
        core.workbench.document(doc).unwrap().buffer.text(),
        "Model one\n"
    );
    // Undo restores the open document.
    common::run(&mut app, "search.undoReplace", Value::Null);
    let core = &app.harness.state().core;
    assert_eq!(
        core.workbench.document(doc).unwrap().buffer.text(),
        "MCP one\n"
    );
}

fn quick_open(app: &mut common::TestApp, text: &str) {
    common::run(app, "navigate.quickOpen", Value::Null);
    app.harness.run_steps(2);
    for c in text.chars() {
        app.harness.event(Event::Text(c.to_string()));
        app.harness.run_steps(1);
    }
    app.harness.run_steps(1);
}

fn ws_indexed(root: &Path) -> common::TestApp {
    let mut app = open_ws(root, vec![]);
    assert!(wait_for(&mut app, 10, |a| a
        .harness
        .state()
        .core
        .workspace
        .as_ref()
        .is_some_and(|w| w.index_ready)));
    app
}

// spec 15: "Fuzzy file search"
#[test]
fn quick_open_fuzzy() {
    let (_d, root) = folder();
    for f in [
        "architecture.md",
        "architecture-old.md",
        "docs/architecture/design.md",
        "README.md",
    ] {
        write(&root, f, "x\n");
    }
    let mut app = ws_indexed(&root);
    quick_open(&mut app, "arch");
    let items: Vec<String> = app
        .harness
        .state()
        .ui
        .picker
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|i| format!("{}/{}", i.detail, i.label))
        .collect();
    assert_eq!(items.len(), 3, "{items:?}");
    let mut sorted = items.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        [
            "/architecture-old.md",
            "/architecture.md",
            "docs/architecture/design.md"
        ]
    );
    assert!(!items.iter().any(|i| i.contains("README")));
}

// spec 15: "Open at line"
#[test]
fn quick_open_at_line() {
    let (_d, root) = folder();
    write(&root, "README.md", &lines(200, &[]));
    let mut app = ws_indexed(&root);
    quick_open(&mut app, "README.md:120");
    app.harness.key_press(Key::Enter);
    assert!(wait_for(&mut app, 5, |a| active_path(a)
        .is_some_and(|p| p.ends_with("README.md"))));
    assert_eq!(caret(&app).0, 119);
}

// spec 15: "Command prefix"
#[test]
fn quick_open_command_prefix() {
    let (_d, root) = folder();
    let mut app = ws_indexed(&root);
    quick_open(&mut app, ">");
    app.harness.run_steps(2);
    let p = app.harness.state().ui.picker.as_ref().unwrap();
    assert_eq!(p.kind, notoir::shell::overlays::PickerKind::Commands);
    // Typing goes on filtering commands; deleting ">" returns to Quick Open.
    app.harness.key_press(Key::Backspace);
    app.harness.run_steps(2);
    assert_eq!(
        app.harness.state().ui.picker.as_ref().unwrap().kind,
        notoir::shell::overlays::PickerKind::QuickOpen
    );
}

// spec 15: "Back and forward across files"
#[test]
fn back_and_forward_across_files() {
    let (_d, root) = folder();
    let body = lines(150, &[]);
    for f in ["README.md", "config.json", "architecture.md"] {
        write(&root, f, &body);
    }
    let mut app = open_ws(&root, vec![]);
    let go = |app: &mut common::TestApp, f: &str, line: u64| {
        common::run(
            app,
            "quickOpen.open",
            json!({ "path": root.join(f).to_string_lossy(), "line": line }),
        );
        let f = f.to_owned();
        assert!(wait_for(app, 5, |a| active_path(a)
            .is_some_and(|p| p.ends_with(&f))
            && caret(a).0 + 1 == line as usize));
    };
    go(&mut app, "README.md", 92);
    go(&mut app, "config.json", 44);
    go(&mut app, "architecture.md", 10);
    common::run(&mut app, "navigate.back", Value::Null);
    common::settle(&mut app);
    assert!(active_path(&app).unwrap().ends_with("config.json"));
    common::run(&mut app, "navigate.back", Value::Null);
    common::settle(&mut app);
    assert!(active_path(&app).unwrap().ends_with("README.md"));
    assert_eq!(caret(&app).0, 91, "README.md line 92");
    common::run(&mut app, "navigate.forward", Value::Null);
    common::settle(&mut app);
    assert!(active_path(&app).unwrap().ends_with("config.json"));
    assert_eq!(caret(&app).0, 43, "config.json line 44");
}

#[test]
fn jump_within_document_is_recorded() {
    let (_d, root) = folder();
    let f = write(&root, "a.md", &lines(100, &[]));
    let mut app = open_ws(&root, vec![f]);
    common::run(&mut app, "navigate.goToLine", Value::Null);
    for c in "80".chars() {
        app.harness.event(Event::Text(c.to_string()));
        app.harness.run_steps(1);
    }
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    assert_eq!(caret(&app).0, 79);
    common::run(&mut app, "navigate.back", Value::Null);
    assert_eq!(caret(&app).0, 0);
    common::run(&mut app, "navigate.forward", Value::Null);
    assert_eq!(caret(&app).0, 79);
}
