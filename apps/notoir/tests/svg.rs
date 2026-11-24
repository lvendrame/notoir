mod common;
use egui_kittest::kittest::Queryable;
use notoir_core::view::ViewKind;
use notoir_editor::{ChangeSet, EditKind, Selection};
use serde_json::json;
use std::time::{Duration, Instant};
const SOURCE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200" viewBox="0 0 320 200">
  <rect x="16" y="16" width="288" height="168" rx="24" fill="#6750a4"/>
  <circle cx="160" cy="100" r="48" fill="#82d0c8"/>
  <text x="160" y="108" font-family="Manrope" font-size="22" text-anchor="middle" fill="#142326">SVG</text>
</svg>"##;
fn setup() -> (tempfile::TempDir, common::TestApp) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drawing.SVG");
    std::fs::write(&path, SOURCE).unwrap();
    let mut app = common::start(|o| o.open_paths = vec![(path, None)]);
    common::settle(&mut app);
    (dir, app)
}
fn view(app: &common::TestApp) -> notoir_core::ViewId {
    app.harness.state().core.workbench.active_view().unwrap()
}
fn wait(app: &mut common::TestApp, error: bool) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        common::settle(app);
        let c = &app.harness.state().core;
        let v = view(app);
        let revision = c
            .workbench
            .document(c.workbench.view(v).unwrap().document_id)
            .unwrap()
            .revision;
        if c.svg
            .as_ref()
            .and_then(|s| s.views.get(&v))
            .is_some_and(|p| {
                if error {
                    p.error.is_some()
                } else {
                    p.revision == Some(revision) && p.texture.is_some() && p.error.is_none()
                }
            })
        {
            return;
        }
        assert!(Instant::now() < until, "SVG preview timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn replace(app: &mut common::TestApp, text: &str) {
    let core = &mut app.harness.state_mut().core;
    let v = core.workbench.active_view().unwrap();
    let id = core.workbench.active_document().unwrap();
    let len = core.workbench.document(id).unwrap().buffer.len_chars();
    core.workbench
        .apply_edit(
            id,
            Some(v),
            ChangeSet::replace(0, len, text),
            Selection::caret(0),
            EditKind::Other,
            notoir_core::ChangeOrigin::User { view: v },
            core.now_ms,
        )
        .unwrap();
}
// spec 43: "SVG uses Edit Split and Preview without changing source"
#[test]
fn modes_share_editable_source() {
    let (_dir, mut app) = setup();
    let v = view(&app);
    assert_eq!(
        app.harness.state().core.workbench.view(v).unwrap().kind,
        ViewKind::Text
    );
    assert!(app.harness.state().core.svg.is_none());
    app.harness.get_by_label("Split").click();
    app.harness.run_steps(3);
    wait(&mut app, false);
    assert_eq!(
        app.harness.state().core.workbench.view(v).unwrap().kind,
        ViewKind::SvgSplit
    );
    app.harness.get_by_label("Preview").click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.workbench.view(v).unwrap().kind,
        ViewKind::SvgPreview
    );
    app.harness
        .query_all_by_label("Edit")
        .last()
        .unwrap()
        .click();
    app.harness.run_steps(3);
    let c = &app.harness.state().core;
    let doc = c
        .workbench
        .document(c.workbench.active_document().unwrap())
        .unwrap();
    assert_eq!(c.workbench.view(v).unwrap().kind, ViewKind::Text);
    assert_eq!(doc.buffer.text(), SOURCE);
    assert!(!doc.is_dirty());
    assert!(!doc.is_read_only());
    assert!(c.svg.is_none());
}
// spec 43: "SVG preview follows unsaved changes and recovers from invalid source"
#[test]
fn preview_tracks_unsaved_edits_errors_and_undo() {
    let (dir, mut app) = setup();
    common::run(&mut app, "svg.showSplit", json!({}));
    wait(&mut app, false);
    let v = view(&app);
    replace(&mut app, &SOURCE.replace("#6750a4", "#dd5544"));
    wait(&mut app, false);
    let revision = app.harness.state().core.svg.as_ref().unwrap().views[&v]
        .revision
        .unwrap();
    assert!(revision > 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("drawing.SVG")).unwrap(),
        SOURCE
    );
    replace(&mut app, "<svg>");
    wait(&mut app, true);
    let p = &app.harness.state().core.svg.as_ref().unwrap().views[&v];
    assert!(p.texture.is_some());
    assert_eq!(p.revision, Some(revision));
    common::run(&mut app, "edit.undo", json!({}));
    wait(&mut app, false);
    assert!(
        app.harness.state().core.svg.as_ref().unwrap().views[&v]
            .error
            .is_none()
    );
}
// spec 43: "SVG presentation survives session restoration"
#[test]
fn session_and_zoom_are_independent_of_text() {
    let (_dir, mut app) = setup();
    common::run(&mut app, "svg.showSplit", json!({}));
    wait(&mut app, false);
    let v = view(&app);
    app.harness
        .state_mut()
        .core
        .workbench
        .views
        .get_mut(v)
        .unwrap()
        .zoom = Some(125.0);
    common::run(&mut app, "svg.setZoom", json!({"view":v.0,"value":150}));
    wait(&mut app, false);
    app.harness
        .state_mut()
        .core
        .workbench
        .views
        .get_mut(v)
        .unwrap()
        .preview_ratio = 0.65;
    let data = app.harness.state().core.capture_session();
    common::run(&mut app, "file.close", json!({}));
    app.harness.state_mut().core.apply_session(data);
    common::settle(&mut app);
    wait(&mut app, false);
    let core = &app.harness.state().core;
    let restored = core.workbench.view(view(&app)).unwrap();
    assert_eq!(restored.kind, ViewKind::SvgSplit);
    assert_eq!(restored.zoom, Some(125.0));
    assert_eq!(restored.svg.as_ref().unwrap().zoom, Some(150.0));
    assert_eq!(restored.preview_ratio, 0.65);
    common::run(&mut app, "svg.showEditor", json!({}));
    assert!(app.harness.state().core.svg.is_none());
}
#[test]
fn plain_xml_has_no_svg_modes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("plain.xml");
    std::fs::write(&p, "<root/>").unwrap();
    let mut app = common::start(|o| o.open_paths = vec![(p, None)]);
    common::settle(&mut app);
    assert!(
        !app.harness.state().core.is_svg(
            app.harness
                .state()
                .core
                .workbench
                .active_document()
                .unwrap()
        )
    );
    assert!(app.harness.query_by_label("Preview").is_none());
}
#[test]
fn svg_snapshots() {
    for theme in notoir::theme::ThemeId::ALL {
        let (_dir, mut app) = setup();
        app.harness.state_mut().core.theme_override = Some(theme);
        for (name, command, width) in [
            ("split", "svg.showSplit", 1280.0),
            ("preview", "svg.showPreview", 1280.0),
            ("narrow", "svg.showSplit", 560.0),
        ] {
            app.harness.set_size(egui::vec2(width, 800.0));
            common::run(&mut app, command, json!({}));
            wait(&mut app, false);
            app.harness.run_steps(3);
            let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/ui-snapshots");
            std::fs::create_dir_all(&out).unwrap();
            if let Ok(image) = app.harness.render() {
                image
                    .save(out.join(format!("svg-{name}-{}.png", theme.id())))
                    .unwrap();
            }
        }
    }
}

#[test]
fn save_from_preview_uses_unsaved_source() {
    let (dir, mut app) = setup();
    let source = SOURCE.replace("#6750a4", "#dd5544");
    replace(&mut app, &source);
    common::run(&mut app, "svg.showPreview", json!({}));
    wait(&mut app, false);
    common::run(&mut app, "file.save", json!({}));
    common::settle(&mut app);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("drawing.SVG")).unwrap(),
        source
    );
    let core = &app.harness.state().core;
    assert!(
        !core
            .workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .is_dirty()
    );
}
#[test]
fn rapid_edits_show_only_current_revision() {
    let (_dir, mut app) = setup();
    common::run(&mut app, "svg.showPreview", json!({}));
    for n in 0..12 {
        replace(&mut app, &SOURCE.replace(">SVG<", &format!(">SVG {n}<")));
        app.harness.run_steps(1);
    }
    wait(&mut app, false);
    let core = &app.harness.state().core;
    let id = core.workbench.active_document().unwrap();
    let revision = core.workbench.document(id).unwrap().revision;
    assert_eq!(
        core.svg.as_ref().unwrap().views[&view(&app)].revision,
        Some(revision)
    );
    assert!(
        core.workbench
            .document(id)
            .unwrap()
            .buffer
            .text()
            .contains("SVG 11")
    );
}

#[test]
fn language_override_disables_svg_controls() {
    let (_dir, mut app) = setup();
    let core = &mut app.harness.state_mut().core;
    let id = core.workbench.active_document().unwrap();
    let doc = core.workbench.document_mut(id).unwrap();
    doc.language = notoir_editor::LanguageId::PLAINTEXT;
    doc.language_overridden = true;
    app.harness.run_steps(3);
    assert!(!app.harness.state().core.is_svg(id));
    assert!(app.harness.query_by_label("Preview").is_none());
    assert!(app.harness.state().core.svg.is_none());
}
