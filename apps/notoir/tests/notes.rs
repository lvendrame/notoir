//! Notes (spec 34): following wiki links (headings, ambiguity, missing
//! notes), `[[` completion, backlinks and unlinked mentions, tags and Quick
//! Open `#`, link updates after a rename, editor decorations.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Event;
use egui_kittest::kittest::Queryable;
use notoir::files::Prompt;
use notoir::notes::NotesPrompt;
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

const NOTES: &[(&str, &str)] = &[
    (
        "Architecture Decisions.md",
        "# Architecture Decisions\n\nIntro.\n\n## MCP\n\nThe MCP part. #architecture\n",
    ),
    (
        "Project Plan.md",
        "# Project Plan\n\nWe decided in [[Architecture Decisions]].\nSee [[Architecture Decisions#MCP|the MCP section]].\nAlso [[Missing Note]] and [[Dup]].\nTags: #plan #rust\n",
    ),
    (
        "notes/Meeting.md",
        "Talked about Architecture Decisions today. #meeting #rust\n",
    ),
    ("a/Dup.md", "first\n"),
    ("b/Dup.md", "second\n"),
];

/// A workspace of notes; `open` is opened in a tab once the index is ready.
fn notes_app(open: &str) -> (tempfile::TempDir, PathBuf, common::TestApp) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    for (rel, text) in NOTES {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let paths = vec![(root.clone(), None), (root.join(open), None)];
    let mut app = common::start(move |o| o.open_paths = paths);
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        core.notes.ready && core.workbench.active_document().is_some()
    }));
    (d, root, app)
}

fn active_path(app: &common::TestApp) -> Option<PathBuf> {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document()?;
    core.workbench.document(doc)?.path().map(Path::to_path_buf)
}

fn active_text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

/// Ctrl/Cmd+click on the first occurrence of `needle` (what the editor
/// sends for that click).
fn click_link(app: &mut common::TestApp, needle: &str) {
    let core = &app.harness.state().core;
    let view = core.workbench.active_editor().unwrap();
    let text = active_text(app);
    let byte = text.find(needle).unwrap();
    let offset = text[..byte].chars().count() + 3;
    common::run(
        app,
        "notes.followLink",
        json!({ "view": view.0, "offset": offset }),
    );
}

fn caret_line(app: &common::TestApp) -> usize {
    let core = &app.harness.state().core;
    let view = core
        .workbench
        .view(core.workbench.active_editor().unwrap())
        .unwrap();
    let rope = core
        .workbench
        .document(view.document_id)
        .unwrap()
        .buffer
        .rope();
    rope.char_to_line(view.selection.primary().head)
}

// spec 34: "Follow a wiki link"
#[test]
fn follow_a_wiki_link() {
    let (_d, root, mut app) = notes_app("Project Plan.md");
    // Find where the link is on screen: put the caret inside it.
    let text = active_text(&app);
    let offset = text[..text.find("[[Architecture Decisions]]").unwrap()]
        .chars()
        .count()
        + 5;
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    let _ = core
        .workbench
        .set_selection(view, notoir_editor::Selection::caret(offset));
    core.effects.push(notoir::core::Effect::FocusEditor(view));
    app.harness.run_steps(3);
    let caret = app.harness.state().ui.editors.caret_rect(view).unwrap();
    let pos = caret.center();
    // Ctrl/Cmd+click there.
    let modifiers = egui::Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        mac_cmd: cfg!(target_os = "macos"),
        command: true,
        ..Default::default()
    };
    app.harness.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        app.harness.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
        app.harness.step();
    }
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("Architecture Decisions.md"))));
}

#[test]
fn heading_links_ambiguous_and_missing_targets() {
    let (_d, root, mut app) = notes_app("Project Plan.md");
    // `#MCP` lands on the heading.
    click_link(&mut app, "[[Architecture Decisions#MCP");
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("Architecture Decisions.md"))));
    assert!(wait_for(&mut app, |a| caret_line(a) == 4));
    // Back to the plan: two notes named Dup → a picker.
    app.harness.state_mut().core.open_path(
        &root.join("Project Plan.md"),
        notoir::files::OpenAt::default(),
    );
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("Project Plan.md"))));
    click_link(&mut app, "[[Dup]]");
    app.harness.run_steps(2);
    let picker = &app.harness.state().core.notes.picker;
    assert_eq!(
        picker.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
        ["a/Dup.md", "b/Dup.md"]
    );
    common::run(&mut app, "notes.pick", json!({ "value": "b/Dup.md" }));
    assert!(wait_for(&mut app, |a| active_path(a) == Some(root.join("b/Dup.md"))));
    // A missing note: asked, then created next to the linking note.
    app.harness.state_mut().core.open_path(
        &root.join("Project Plan.md"),
        notoir::files::OpenAt::default(),
    );
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("Project Plan.md"))));
    click_link(&mut app, "[[Missing Note]]");
    let prompt = app.harness.state().core.files.prompts.front().cloned();
    assert!(
        matches!(&prompt, Some(Prompt::Notes(NotesPrompt::Create { name, .. })) if name == "Missing Note"),
        "{prompt:?}"
    );
    app.harness.run_steps(2);
    app.harness.get_by_label("Create Note").click();
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("Missing Note.md"))));
    assert!(root.join("Missing Note.md").is_file());
}

#[test]
fn editor_marks_links_and_missing_targets() {
    let (_d, _root, mut app) = notes_app("Project Plan.md");
    let core = &mut app.harness.state_mut().core;
    let doc = core.workbench.active_document().unwrap();
    let links = core.notes_doc_links(doc);
    let missing: Vec<bool> = links.iter().map(|l| l.2).collect();
    // Architecture Decisions ×2, Missing Note, Dup (ambiguous, not missing).
    assert_eq!(missing, [false, false, true, false]);
}

#[test]
fn completion_after_double_brackets() {
    let (_d, _root, mut app) = notes_app("notes/Meeting.md");
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    let end = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .len_chars();
    let _ = core
        .workbench
        .set_selection(view, notoir_editor::Selection::caret(end));
    core.effects.push(notoir::core::Effect::FocusEditor(view));
    app.harness.run_steps(2);
    app.harness.event(Event::Text("See [[proj".into()));
    app.harness.run_steps(3);
    let items: Vec<String> = app
        .harness
        .state()
        .core
        .notes
        .completion
        .as_ref()
        .map(|c| c.items.iter().map(|i| i.0.clone()).collect())
        .unwrap_or_default();
    assert_eq!(
        items.first().map(String::as_str),
        Some("Project Plan"),
        "{items:?}"
    );
    app.harness
        .query_all_by_label_contains("Project Plan")
        .next()
        .expect("popup row");
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert!(
        active_text(&app).ends_with("See [[Project Plan]]"),
        "{}",
        active_text(&app)
    );
    assert!(app.harness.state().core.notes.completion.is_none());
    // Undo removes the completion.
    common::run(&mut app, "edit.undo", json!({}));
    assert!(
        active_text(&app).ends_with("See [[proj"),
        "{}",
        active_text(&app)
    );
}

#[test]
fn backlinks_and_unlinked_mentions() {
    let (_d, _root, mut app) = notes_app("Architecture Decisions.md");
    common::run(&mut app, "notes.showBacklinks", json!({}));
    app.harness.run_steps(2);
    app.harness.get_by_label("Referenced by (2):");
    app.harness
        .query_all_by_label_contains("We decided in [[Architecture Decisions]].")
        .next()
        .expect("context snippet");
    common::run(
        &mut app,
        "notes.findUnlinked",
        json!({ "note": "Architecture Decisions.md" }),
    );
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .notes
        .unlinked
        .is_some()));
    let (_, found) = app.harness.state().core.notes.unlinked.clone().unwrap();
    assert_eq!(
        found.iter().map(|f| f.source.as_str()).collect::<Vec<_>>(),
        ["notes/Meeting.md"]
    );
}

#[test]
fn edits_update_backlinks() {
    let (_d, _root, mut app) = notes_app("notes/Meeting.md");
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    let _ = core
        .workbench
        .set_selection(view, notoir_editor::Selection::caret(0));
    core.effects.push(notoir::core::Effect::FocusEditor(view));
    app.harness.run_steps(2);
    app.harness
        .event(Event::Text("Link: [[Project Plan]] ".into()));
    app.harness.run_steps(2);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .notes
        .index
        .backlinks("Project Plan.md")
        .iter()
        .any(|b| b.source == "notes/Meeting.md")));
}

#[test]
fn tags_panel_and_quick_open_hash() {
    let (_d, root, mut app) = notes_app("Project Plan.md");
    let tags = app.harness.state().core.notes.index.tags();
    assert_eq!(
        tags,
        [
            ("architecture".to_owned(), 1),
            ("meeting".to_owned(), 1),
            ("plan".to_owned(), 1),
            ("rust".to_owned(), 2)
        ]
    );
    common::run(&mut app, "notes.showTags", json!({}));
    app.harness.get_by_label("#rust  2").click();
    app.harness.run_steps(3);
    app.harness
        .query_all_by_label_contains("Meeting")
        .next()
        .expect("notes with the tag");
    // Quick Open `#rust` → the tag → its notes.
    common::run(&mut app, "navigate.quickOpen", json!({}));
    app.harness.event(Event::Text("#ru".into()));
    app.harness.run_steps(3);
    app.harness
        .query_all_by_label_contains("#rust")
        .next()
        .expect("tag row");
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(3);
    let rows: Vec<String> = app
        .harness
        .state()
        .core
        .notes
        .picker
        .iter()
        .map(|p| p.0.clone())
        .collect();
    assert_eq!(rows, ["Project Plan.md", "notes/Meeting.md"]);
    app.harness.key_press(egui::Key::Enter);
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("Project Plan.md"))));
}

#[test]
fn rename_offers_to_update_links() {
    let (_d, root, mut app) = notes_app("Project Plan.md");
    // Rename through the Explorer.
    common::run(
        &mut app,
        "explorer.rename",
        json!({ "path": root.join("Architecture Decisions.md").to_string_lossy() }),
    );
    let core = &mut app.harness.state_mut().core;
    core.workspace.as_mut().unwrap().edit.as_mut().unwrap().text = "Decisions.md".into();
    core.commit_edit();
    assert!(wait_for(&mut app, |a| matches!(
        a.harness.state().core.files.prompts.front(),
        Some(Prompt::Notes(NotesPrompt::UpdateLinks { .. }))
    )));
    app.harness.run_steps(2);
    app.harness
        .query_all_by_label_contains("Update links to “Architecture Decisions” in 1 notes?")
        .next()
        .expect("rename prompt");
    app.harness.get_by_label("Update Links").click();
    app.harness.run_steps(3);
    // The open note changed in its buffer (undoable), not on disk yet.
    let text = active_text(&app);
    assert!(
        text.contains("[[Decisions]]") && text.contains("[[Decisions#MCP|the MCP section]]"),
        "{text}"
    );
    assert!(
        std::fs::read_to_string(root.join("Project Plan.md"))
            .unwrap()
            .contains("[[Architecture Decisions]]")
    );
    common::run(&mut app, "edit.undo", json!({}));
    assert!(active_text(&app).contains("[[Architecture Decisions]]"));
}
