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
    let png = dir.path().join("dot #%25(1).png");
    image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]))
        .save(&png)
        .unwrap();
    let mut app = open(vec![write(
        dir.path(),
        "img.md",
        "![dot](dot%20%23%2525%281%29.png) and ![gone](missing.png)\n",
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

/// A pane narrower than the minimum column scrolls sideways instead of
/// squeezing the text into a sliver.
#[test]
fn narrow_preview_scrolls_sideways_instead_of_squeezing() {
    let dir = tempfile::tempdir().unwrap();
    let words: Vec<String> = (0..60).map(|i| format!("w{i}")).collect();
    let md = format!("# Narrow\n\n{}\n", words.join(" "));
    let mut app = open(vec![write(dir.path(), "narrow.md", &md)]);
    common::run(&mut app, "markdown.showSplit", Value::Null);
    // The Explorer leaves the preview about 220 px.
    app.harness.set_size(egui::vec2(520.0, 800.0));
    app.harness.run_steps(4);
    let rects: Vec<egui::Rect> = words
        .iter()
        .filter_map(|w| {
            let label = format!("{w} ");
            app.harness.query_by_label(&label).map(|n| n.rect())
        })
        .collect();
    assert!(!rects.is_empty());
    let row = rects.iter().map(|r| r.right()).fold(f32::MIN, f32::max)
        - rects.iter().map(|r| r.left()).fold(f32::MAX, f32::min);
    assert!(row >= 300.0, "rows keep a readable width: {row}");
    let before = app.harness.get_by_label("Narrow").rect();
    app.harness
        .input_mut()
        .events
        .push(Event::PointerMoved(before.center()));
    app.harness.run_steps(2);
    app.harness.input_mut().events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-500.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(30);
    let after = app.harness.get_by_label("Narrow").rect();
    assert!(after.left() < before.left(), "{before:?} → {after:?}");
}

fn select_md(app: &mut common::TestApp, a: usize, b: usize) {
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_view().unwrap();
    core.workbench
        .set_selection(
            view,
            notoir_editor::Selection::single(notoir_editor::Range::new(a, b)),
        )
        .unwrap();
    app.harness.run_steps(2);
}

// spec 24: "Insert Markdown blocks as undoable edits"
#[test]
fn insert_blocks_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "café\nnext\n")]);
    for (args, expected) in [
        (json!({"kind":"heading","level":3}), "### café\n### next\n"),
        (json!({"kind":"quote"}), "> café\n> next\n"),
        (json!({"kind":"bullet"}), "- café\n- next\n"),
        (json!({"kind":"numbered"}), "1. café\n2. next\n"),
        (json!({"kind":"task"}), "- [ ] café\n- [ ] next\n"),
        (
            json!({"kind":"code","language":"typescript"}),
            "```typescript\ncafé\nnext\n```\n\n",
        ),
        (
            json!({"kind":"code","language":"shellscript"}),
            "```sh\ncafé\nnext\n```\n\n",
        ),
    ] {
        select_md(&mut app, 0, 10);
        common::run(&mut app, "markdown.insertBlock", args);
        assert_eq!(text(&app), expected);
        common::run(&mut app, "edit.undo", Value::Null);
        assert_eq!(text(&app), "café\nnext\n");
        common::run(&mut app, "edit.redo", Value::Null);
        assert_eq!(text(&app), expected);
        common::run(&mut app, "edit.undo", Value::Null);
    }
}

// spec 24: "Insert link through a dialog"
#[test]
fn link_dialog_confirm_cancel_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "café")]);
    select_md(&mut app, 0, 4);
    common::run(&mut app, "markdown.insertLink", Value::Null);
    assert_eq!(text(&app), "café");
    assert_eq!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .caption,
        "café"
    );
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    assert!(app.harness.state().core.markdown.insert_dialog.is_none());
    assert_eq!(text(&app), "café");
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap()
        .destination = "https://example.com/a(b)".into();
    app.harness.run_steps(2);
    app.harness
        .get_by_role_and_label(Role::Button, "Insert")
        .click();
    app.harness.run_steps(4);
    assert_eq!(text(&app), "[café](https://example.com/a%28b%29)");
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), "café");
}

// spec 24: "Complete workspace files in link destinations"
#[test]
fn workspace_link_completion_keyboard_and_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    let source = write(dir.path(), "docs/source.md", "caption");
    write(dir.path(), "target #1.md", "# Target");
    write(dir.path(), "other.txt", "other");
    let mut app = open(vec![dir.path().to_owned(), source]);
    for _ in 0..100 {
        if app
            .harness
            .state()
            .core
            .workspace
            .as_ref()
            .is_some_and(|w| w.index_ready)
        {
            break;
        }
        common::settle(&mut app);
        std::thread::sleep(Duration::from_millis(10));
    }
    select_md(&mut app, 0, 7);
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness
        .get_by_role_and_label(Role::TextInput, "URL or file")
        .click();
    app.harness.event(Event::Text("@target".into()));
    app.harness.run_steps(3);
    assert!(
        app.harness
            .query_all_by_label("target #1.md")
            .next()
            .is_some()
    );
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .destination,
        "../target %231.md"
    );
    assert_eq!(text(&app), "caption", "choosing a file does not submit");
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(4);
    assert_eq!(text(&app), "[caption](../target%20%231.md)");
    let source = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    common::run(
        &mut app,
        "markdown.openLink",
        json!({"doc": source.0, "url":"../target%20%231.md"}),
    );
    common::settle(&mut app);
    assert_eq!(text(&app), "# Target");
}

// spec 24: "Reject a stale Markdown insertion dialog"
#[test]
fn stale_link_dialog_does_not_edit_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "hello")]);
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap()
        .destination = "https://example.com".into();
    common::run(
        &mut app,
        "markdown.insertBlock",
        json!({"kind":"heading","level":1}),
    );
    let before = text(&app);
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(text(&app), before);
    assert!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .error
    );
    assert!(
        app.harness
            .query_by_label(
                "The original document changed or is no longer editable. Cancel and try again."
            )
            .is_some()
    );
}

#[test]
fn image_and_language_picker() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "")]);
    common::run(&mut app, "markdown.insertImage", Value::Null);
    let d = app
        .harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap();
    d.destination = "image.png".into();
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(text(&app), "![](image.png)");
    common::run(&mut app, "edit.undo", Value::Null);
    common::run(&mut app, "markdown.insertCodePicker", Value::Null);
    app.harness.event(Event::Text("Python".into()));
    app.harness.run_steps(3);
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(4);
    assert_eq!(text(&app), "```python\n\n```\n\n");
}

#[test]
fn markdown_actions_target_view_and_respect_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![
        write(dir.path(), "a.md", "a"),
        write(dir.path(), "b.md", "b"),
    ]);
    let core = &mut app.harness.state_mut().core;
    let target = core
        .workbench
        .views
        .iter()
        .find(|v| {
            core.workbench
                .document(v.document_id)
                .unwrap()
                .buffer
                .text()
                == "a"
        })
        .unwrap()
        .id;
    let doc = core.workbench.view(target).unwrap().document_id;
    common::run(
        &mut app,
        "markdown.insertBlock",
        json!({"view":target.0,"kind":"quote"}),
    );
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .document(doc)
            .unwrap()
            .buffer
            .text(),
        "> a"
    );
    assert_eq!(text(&app), "b");
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .read_only = notoir_core::document::ReadOnly::User;
    common::run(
        &mut app,
        "markdown.insertBlock",
        json!({"view":target.0,"kind":"quote"}),
    );
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .document(doc)
            .unwrap()
            .buffer
            .text(),
        "> a"
    );
}

fn md_right_click(app: &mut common::TestApp, point: egui::Pos2) {
    app.harness.hover_at(point);
    app.harness.run_steps(1);
    for pressed in [true, false] {
        app.harness.event(Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    app.harness.run_steps(3);
}

// spec 18: "Markdown Insert context menu"
#[test]
fn markdown_context_insert_menu() {
    use egui_kittest::kittest::{By, NodeT};
    // A breadcrumb containing H1-H7 must never match a heading menu item.
    let dir = tempfile::Builder::new()
        .prefix("H1-H2-H3-H4-H5-H6-H7-")
        .tempdir()
        .unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "hello world")]);
    select_md(&mut app, 0, 0);
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let point = app
        .harness
        .state()
        .ui
        .editors
        .caret_rect(v)
        .unwrap()
        .center();
    select_md(&mut app, 0, 5);
    md_right_click(&mut app, point + egui::vec2(8.0, 0.0));
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(v)
            .unwrap()
            .selection
            .primary(),
        notoir_editor::Range::new(0, 5)
    );
    app.harness.get_by_label_contains("Insert").hover();
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label_contains("Link…").is_some());
    assert!(app.harness.query_by_label_contains("Quote").is_some());
    app.harness.get_by_label_contains("Heading").hover();
    app.harness.run_steps(3);
    let heading = |level| {
        let label = format!("H{level}");
        By::new()
            .role(Role::Button)
            .predicate(move |node| node.label().is_some_and(|text| text.trim() == label))
    };
    for level in 1..=6 {
        assert!(app.harness.query(heading(level)).is_some());
    }
    assert!(app.harness.query(heading(7)).is_none());
    app.harness.get(heading(2)).click();
    app.harness.run_steps(4);
    assert_eq!(text(&app), "## hello world");
    // Read-only Markdown retains the menu with disabled insertions.
    let core = &mut app.harness.state_mut().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document_mut(doc).unwrap().read_only = notoir_core::document::ReadOnly::User;
    md_right_click(&mut app, point);
    app.harness.get_by_label_contains("Insert").hover();
    app.harness.run_steps(3);
    assert!(
        app.harness
            .get_by_label_contains("Link…")
            .accesskit_node()
            .is_disabled()
    );
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    common::run(
        &mut app,
        "editor.changeLanguageMode",
        json!({"language":"plaintext"}),
    );
    md_right_click(&mut app, point);
    assert!(app.harness.query_by_label_contains("Insert").is_none());
}

#[test]
fn markdown_right_click_outside_selection_moves_caret() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "hello world")]);
    select_md(&mut app, 8, 8);
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let point = app
        .harness
        .state()
        .ui
        .editors
        .caret_rect(v)
        .unwrap()
        .center();
    select_md(&mut app, 0, 5);
    md_right_click(&mut app, point);
    let range = app
        .harness
        .state()
        .core
        .workbench
        .view(v)
        .unwrap()
        .selection
        .primary();
    assert!(range.is_empty());
    assert!((7..=9).contains(&range.head));
    app.harness.get_by_label_contains("Insert").hover();
    app.harness.run_steps(3);
    app.harness.get_by_label_contains("Quote").click();
    app.harness.run_steps(4);
    assert_eq!(text(&app), "> hello world");
}

#[test]
fn unsaved_file_links_and_completion_empty_states() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![]);
    common::run(&mut app, "file.newUntitled", Value::Null);
    common::run(
        &mut app,
        "editor.changeLanguageMode",
        json!({"language":"markdown"}),
    );
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap()
        .destination = "@".into();
    app.harness.run_steps(3);
    assert!(
        app.harness
            .query_by_label("Open a workspace to search files.")
            .is_some()
    );
    assert!(
        app.harness
            .state()
            .core
            .md_file_candidates("", false)
            .is_empty()
    );
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    let file = write(dir.path(), "target.md", "target");
    app.harness.state_mut().core.open_workspace(dir.path());
    for _ in 0..100 {
        common::settle(&mut app);
        if app
            .harness
            .state()
            .core
            .workspace
            .as_ref()
            .is_some_and(|w| w.index_ready)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness.state_mut().core.choose_md_file("target.md");
    let dialog = app
        .harness
        .state()
        .core
        .markdown
        .insert_dialog
        .as_ref()
        .unwrap();
    assert_eq!(dialog.destination, "target.md");
    assert_eq!(dialog.caption, "target.md");
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(text(&app), "[target.md](target.md)");
    assert!(file.exists());
}

#[test]
fn dialog_refuses_language_readonly_and_closed_target() {
    use notoir_core::document::ReadOnly;
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "hello")]);
    for reason in ["language", "readonly", "closed"] {
        common::run(&mut app, "markdown.insertLink", Value::Null);
        let core = &mut app.harness.state_mut().core;
        let dialog = core.markdown.insert_dialog.as_mut().unwrap();
        dialog.destination = "https://example.com".into();
        let (view, doc) = (dialog.view, dialog.document);
        match reason {
            "language" => {
                core.workbench.document_mut(doc).unwrap().language =
                    notoir_editor::LanguageId::PLAINTEXT
            }
            "readonly" => core.workbench.document_mut(doc).unwrap().read_only = ReadOnly::User,
            _ => {
                core.workbench.close_view(view, true).unwrap();
            }
        }
        core.confirm_md_insert(None);
        assert!(core.markdown.insert_dialog.as_ref().unwrap().error);
        if let Some(d) = core.workbench.document_mut(doc) {
            assert_eq!(d.buffer.text(), "hello");
            d.language = notoir_editor::LanguageId::MARKDOWN;
            d.read_only = ReadOnly::No;
        }
        core.markdown.insert_dialog = None;
    }
}

// spec 24: "Filename matches rank ahead of fuzzy path matches"
#[test]
fn filename_search_prioritizes_exact_names_in_link_and_image_dialogs() {
    use notoir_core::explorer::{IndexEntry, NodeKind};
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![
        dir.path().to_owned(),
        write(dir.path(), "notes.md", ""),
    ]);
    common::settle(&mut app);
    let core = &mut app.harness.state_mut().core;
    let mut entries: Vec<_> = (0..300)
        .map(|i| IndexEntry {
            rel: format!("src/{i}/application_results.rs"),
            kind: NodeKind::File,
            hidden: false,
            ignored: false,
        })
        .collect();
    for path in [
        "apps/notoir/src/app.rs",
        "assets/app.png",
        "assets/application.png",
        "assets/app.png.rs",
    ] {
        entries.push(IndexEntry {
            rel: path.into(),
            kind: NodeKind::File,
            hidden: false,
            ignored: false,
        });
    }
    core.workspace.as_mut().unwrap().index = std::sync::Arc::new(entries);
    core.workspace.as_mut().unwrap().index_ready = true;
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness
        .get_by_role_and_label(Role::TextInput, "URL or file")
        .click();
    app.harness.event(Event::Text("@app.rs".into()));
    app.harness.run_steps(3);
    assert!(
        app.harness
            .query_by_label("apps/notoir/src/app.rs")
            .is_some()
    );
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .destination,
        "apps/notoir/src/app.rs"
    );
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    common::run(&mut app, "markdown.insertImage", Value::Null);
    app.harness
        .get_by_role_and_label(Role::TextInput, "URL or file")
        .click();
    app.harness.event(Event::Text("@app".into()));
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("assets/app.png").is_some());
    assert!(
        app.harness
            .query_by_label("apps/notoir/src/app.rs")
            .is_none()
    );
    assert!(app.harness.query_by_label("assets/app.png.rs").is_none());
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .destination,
        "assets/app.png"
    );
}

// spec 24: "Choose table dimensions before insertion"
#[test]
fn table_dialog_dimensions_cancel_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.md", "")]);
    common::run(&mut app, "markdown.insertBlock", json!({"kind":"table"}));
    assert_eq!(text(&app), "");
    assert!(
        app.harness
            .query_by_role_and_label(Role::SpinButton, "Columns")
            .is_some()
    );
    assert!(
        app.harness
            .query_by_role_and_label(Role::SpinButton, "Data rows")
            .is_some()
    );
    app.harness
        .get_by_role_and_label(Role::Button, "Cancel")
        .click();
    app.harness.run_steps(3);
    assert!(app.harness.state().core.markdown.insert_dialog.is_none());
    assert_eq!(text(&app), "");
    common::run(&mut app, "markdown.insertBlock", json!({"kind":"table"}));
    app.harness
        .get_by_role_and_label(Role::SpinButton, "Columns")
        .click();
    app.harness.run_steps(2);
    app.harness.key_press_modifiers(ctrl(), Key::A);
    app.harness.event(Event::Text("3".into()));
    app.harness.key_press(Key::Tab);
    app.harness.run_steps(3);
    app.harness
        .get_by_role_and_label(Role::SpinButton, "Data rows")
        .click();
    app.harness.run_steps(2);
    app.harness.key_press_modifiers(ctrl(), Key::A);
    app.harness.event(Event::Text("4".into()));
    app.harness.key_press(Key::Tab);
    app.harness.run_steps(3);
    let draft = app
        .harness
        .state()
        .core
        .markdown
        .insert_dialog
        .as_ref()
        .unwrap();
    assert_eq!((draft.columns, draft.rows), (3, 4));
    assert_eq!(text(&app), "");
    app.harness
        .get_by_role_and_label(Role::Button, "Insert")
        .click();
    app.harness.run_steps(4);
    let parsed = notoir_editor::markdown::parse(&text(&app));
    let notoir_editor::markdown::BlockKind::Table { header, rows, .. } = &parsed.blocks[0].kind
    else {
        panic!("expected a Markdown table");
    };
    assert_eq!(header.len(), 3);
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.len() == 3));
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), "");
    common::run(&mut app, "edit.redo", Value::Null);
    assert!(text(&app).contains("| --- | --- | --- |"));
}
