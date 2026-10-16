//! Markdown modes, preview, links, tasks and scroll sync (spec 24), and the
//! language-mode override (spec 26).

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use notoir_core::view::ViewKind;
use serde_json::{Value, json};

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn open(files: Vec<PathBuf>) -> common::TestApp {
    let mut app =
        common::start(move |o| o.open_paths = files.into_iter().map(|f| (f, None)).collect());
    common::settle(&mut app);
    app
}

fn active_kind(app: &common::TestApp) -> ViewKind {
    let core = &app.harness.state().core;
    core.workbench
        .view(core.workbench.active_view().unwrap())
        .unwrap()
        .kind
}

fn text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    core.workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .text()
}

// spec 24: "Switch to split mode"
#[test]
fn switch_to_split_from_toolbar() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(
        dir.path(),
        "README.md",
        "# Notoir\n\nHello **world**.\n",
    )]);
    assert_eq!(active_kind(&app), ViewKind::Text);
    app.harness.get_by_label("Split").click();
    app.harness.run_steps(3);
    assert_eq!(active_kind(&app), ViewKind::MarkdownSplit);
    // Editor (text value) and preview (rendered heading label) side by side.
    assert!(
        app.harness.query_by_label("Notoir").is_some(),
        "preview heading"
    );
    assert!(
        app.harness.state().core.editor_focused
            || app
                .harness
                .query_by_role(Role::MultilineTextInput)
                .is_some()
    );
    let v = app.harness.state().core.workbench.active_view().unwrap();
    assert!(
        app.harness.state().ui.editors.top_line(v).is_some(),
        "editor still rendered"
    );
}

// spec 24: "Live preview"
#[test]
fn live_preview_updates_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "notes.md", "intro\n")]);
    common::run(&mut app, "markdown.showSplit", Value::Null);
    app.harness.key_press_modifiers(ctrl(), Key::End);
    app.harness.event(Event::Text("\n# Title".into()));
    app.harness.run_steps(2);
    std::thread::sleep(Duration::from_millis(110));
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("Title").is_some());
}

// spec 24: "Task list toggle writes to source"
#[test]
fn preview_checkbox_toggles_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "todo.md", "- [ ] buy milk\n")]);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    app.harness.get_by_role(Role::CheckBox).click();
    app.harness.run_steps(3);
    assert_eq!(text(&app), "- [x] buy milk\n");
    common::run(&mut app, "markdown.showEditor", Value::Null);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), "- [ ] buy milk\n");
}

// spec 24: "Remote images not loaded by default"
#[test]
fn remote_images_are_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(
        dir.path(),
        "img.md",
        "![x](https://example.com/a.png)\n",
    )]);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    assert!(app.harness.query_by_label("Load remote images").is_some());
    assert!(
        app.harness
            .query_by_label_contains("https://example.com/a.png")
            .is_some()
    );
    // Nothing was requested: no image entry, no decode job for it.
    assert!(app.harness.state().core.markdown.requested.is_empty());
}

#[test]
fn local_images_load_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("dot.png");
    image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]))
        .save(&png)
        .unwrap();
    let mut app = open(vec![write(
        dir.path(),
        "img.md",
        "![dot](dot.png) and ![gone](missing.png)\n",
    )]);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    common::settle(&mut app);
    app.harness.run_steps(2);
    let images = &app.harness.state().ui.previews.images;
    assert!(
        matches!(images.get(&png.canonicalize().unwrap()), Some(notoir::shell::md_preview::ImageTex::Ready(_, s)) if s.x == 4.0)
    );
    assert!(
        app.harness
            .query_by_label_contains("Image not found: missing.png")
            .is_some()
    );
}

// spec 24: "Relative link opens file at heading"
#[test]
fn relative_link_opens_file_at_heading() {
    let dir = tempfile::tempdir().unwrap();
    let mut b: String = "# B\n\n".to_owned();
    b.extend((0..80).map(|i| format!("filler {i}\n")));
    b.push_str("\n## Setup\n\nsteps\n");
    write(dir.path(), "b.md", &b);
    let mut app = open(vec![write(
        dir.path(),
        "a.md",
        "See [setup](./b.md#setup).\n",
    )]);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    app.harness.get_by_label("setup").click();
    app.harness.run_steps(2);
    common::settle(&mut app);
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    assert_eq!(core.document_title(doc), "b.md");
    let v = core.workbench.active_view().unwrap();
    let rope = core.workbench.document(doc).unwrap().buffer.rope().clone();
    let line = rope.char_to_line(core.workbench.view(v).unwrap().selection.primary().head);
    assert_eq!(
        notoir_editor::display_map::line_text(&rope, line),
        "## Setup"
    );
    assert_eq!(
        app.harness.state().ui.editors.top_line(v),
        Some(line),
        "scrolled to the heading"
    );
}

#[test]
fn web_links_go_to_the_os() {
    let dir = tempfile::tempdir().unwrap();
    let dialogs = notoir::files::ScriptedDialogs::default();
    let d = dialogs.clone();
    let f = write(dir.path(), "a.md", "[site](https://notoir.dev)\n");
    let mut app = common::start(move |o| {
        o.dialogs = Some(std::sync::Arc::new(d));
        o.open_paths = vec![(f, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    app.harness.get_by_label("site").click();
    app.harness.run_steps(2);
    assert_eq!(
        *dialogs.opened.lock().unwrap(),
        vec!["https://notoir.dev".to_string()]
    );
}

// spec 24: "Editor scroll moves preview"
#[test]
fn editor_scroll_moves_preview() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = String::from("# Doc\n\n");
    for s in 0..12 {
        doc.push_str(&format!("## Section {s}\n\n"));
        for p in 0..6 {
            doc.push_str(&format!(
                "Paragraph {p} of section {s} with enough words to take a line.\n\n"
            ));
        }
    }
    doc.push_str("## Roadmap\n\nLater.\n");
    let roadmap_line = doc.lines().position(|l| l == "## Roadmap").unwrap();
    let mut app = open(vec![write(dir.path(), "long.md", &doc)]);
    common::run(&mut app, "markdown.showSplit", Value::Null);
    let v = app.harness.state().core.workbench.active_view().unwrap();
    app.harness.state_mut().core.find.scroll.insert(
        v,
        notoir_core::view::ScrollPosition {
            top_line: roadmap_line,
            ..Default::default()
        },
    );
    app.harness.run_steps(4);
    let st = &app.harness.state().ui.previews.views[&v];
    let block = st
        .blocks
        .iter()
        .find(|b| b.0 == roadmap_line)
        .expect("roadmap block");
    assert!(
        (st.offset - block.2).abs() < 2.0,
        "preview offset {} vs heading {}",
        st.offset,
        block.2
    );
}

#[test]
fn bold_shortcut_and_url_paste() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "word\n")]);
    let sidebar = app.harness.state().core.panels.primary.visible;
    app.harness.key_press_modifiers(ctrl(), Key::B);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "**word**\n");
    assert_eq!(
        app.harness.state().core.panels.primary.visible,
        sidebar,
        "Mod+B formats instead of toggling the sidebar"
    );
    app.harness.key_press_modifiers(ctrl(), Key::B);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "word\n");
    common::run(&mut app, "edit.selectAll", Value::Null);
    let core = &mut app.harness.state_mut().core;
    let v = core.workbench.active_view().unwrap();
    core.workbench
        .set_selection(
            v,
            notoir_editor::Selection::new(vec![notoir_editor::Range::new(0, 4)], 0),
        )
        .unwrap();
    app.harness.event(Event::Paste("https://notoir.dev".into()));
    app.harness.run_steps(2);
    assert_eq!(text(&app), "[word](https://notoir.dev)\n");
}

// spec 26: "Language mode override"
#[test]
fn language_override_to_markdown_shows_controls() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "notes.txt", "# Title\n")]);
    assert!(app.harness.query_by_label("Split").is_none());
    common::run(
        &mut app,
        "editor.changeLanguageMode",
        json!({ "language": "markdown" }),
    );
    app.harness.run_steps(2);
    assert!(
        app.harness.query_by_label("Split").is_some()
            && app.harness.query_by_label("Preview").is_some()
    );
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let spans = app.harness.state().ui.editors.spans(v, 0).unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.kind == notoir_editor::syntax::TokenKind::Heading)
    );
}

#[test]
fn export_as_html() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Notes.md");
    std::fs::write(
        &src,
        "# Weekly Notes\n\nSee [[Plan#Next Steps]].\n\n- [ ] ship\n",
    )
    .unwrap();
    let out = dir.path().join("export/Notes.html");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let dialogs = notoir::files::ScriptedDialogs::default();
    dialogs.answer_save(Some(out.clone()));
    dialogs.answer_save(None);
    let d = dialogs.clone();
    let mut app = common::start(move |o| {
        o.open_paths = vec![(src, None)];
        o.dialogs = Some(std::sync::Arc::new(d));
    });
    common::settle(&mut app);
    common::run(&mut app, "markdown.exportHtml", serde_json::Value::Null);
    common::settle(&mut app);
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<title>Weekly Notes</title>"));
    assert!(html.contains("<h1 id=\"weekly-notes\">Weekly Notes</h1>"));
    assert!(
        html.contains("<a href=\"Plan.html#next-steps\">Plan</a>"),
        "{html}"
    );
    assert!(
        app.harness
            .state()
            .core
            .notifications
            .history()
            .any(|n| n.message.key == "notifications:exported")
    );
    // Cancelling the dialog writes nothing.
    std::fs::remove_file(&out).unwrap();
    common::run(&mut app, "markdown.exportHtml", serde_json::Value::Null);
    common::settle(&mut app);
    assert!(!out.exists());
}

/// Wide content stays inside the preview column: long words wrap within it,
/// and a table wider than the column scrolls sideways instead of being cut.
#[test]
fn wide_content_wraps_or_scrolls_inside_the_column() {
    let dir = tempfile::tempdir().unwrap();
    let long_word = "Supercalifragilisticexpialidociouswordthatkeepsgoingandgoingwithoutanybreakpointatallreallyverylong";
    let md = format!(
        "# Wide\n\nSome text then {long_word} and more.\n\n| Name | Description | Owner | Status | Notes | Created | Updated | Priority |\n|---|---|---|---|---|---|---|---|\n| Recovery | Hot exit keeps unsaved buffers across restarts and crashes | core team | done | Covers scratchpads and untitled files too | 2025-01-10 | 2026-10-01 | high |\n"
    );
    let mut app = open(vec![write(dir.path(), "wide.md", &md)]);
    common::run(&mut app, "markdown.showSplit", Value::Null);
    app.harness.run_steps(3);
    let column_left = app.harness.get_by_label("Wide").rect().left();
    let preview_right = app.harness.ctx.content_rect().right();
    // The long word starts a row of its own at the column's left edge.
    let word = app
        .harness
        .query_all_by_label_contains("Supercalifragilistic")
        .next()
        .expect("long word")
        .rect();
    assert!(
        word.left() >= column_left - 0.5 && word.right() <= preview_right,
        "word {word:?} inside the column from {column_left}"
    );
    // The last column starts out of view, and scrolling the table sideways
    // brings it in.
    let before = app.harness.get_by_label("Priority").rect();
    assert!(before.right() > preview_right - 16.0, "{before:?}");
    let name = app.harness.get_by_label("Name").rect().center();
    app.harness
        .input_mut()
        .events
        .push(Event::PointerMoved(name));
    app.harness.run_steps(2);
    app.harness.input_mut().events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-2000.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(30);
    let after = app.harness.get_by_label("Priority").rect();
    assert!(after.left() < before.left(), "{before:?} → {after:?}");
    assert!(after.right() <= preview_right, "{after:?} in view");
}
