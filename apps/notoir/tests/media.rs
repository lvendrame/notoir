mod common;
use notoir_core::document::{DocumentContent, MediaKind};
use notoir_core::view::ViewKind;
use serde_json::json;
use std::time::{Duration, Instant};
fn opened() -> (common::TestApp, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picture.png");
    image::RgbaImage::from_fn(640, 360, |x, y| {
        image::Rgba([(x * 255 / 639) as u8, (y * 255 / 359) as u8, 160, 255])
    })
    .save(&path)
    .unwrap();
    let mut app = common::start(|_| {});
    app.harness
        .state_mut()
        .core
        .open_path(&path, Default::default());
    common::settle(&mut app);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        app.harness.run_steps(2);
        let core = &app.harness.state().core;
        let ready = core
            .media
            .as_ref()
            .is_some_and(|m| m.sessions.values().any(|s| s.texture.is_some()));
        if ready {
            break;
        }
        assert!(Instant::now() < until, "image did not open");
        std::thread::sleep(Duration::from_millis(5));
    }
    (app, dir, path)
}
// spec 42: "Media tabs have no editable text or save path"
#[test]
fn image_is_readonly_and_save_cannot_overwrite_it() {
    let (mut app, _dir, path) = opened();
    let before = std::fs::read(&path).unwrap();
    let id = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    let doc = app.harness.state().core.workbench.document(id).unwrap();
    assert!(matches!(
        doc.buffer,
        DocumentContent::Media(MediaKind::Image)
    ));
    assert!(doc.is_read_only());
    assert!(!doc.is_dirty());
    assert!(doc.buffer.as_text().is_none());
    let view = app.harness.state().core.workbench.active_view().unwrap();
    assert_eq!(
        app.harness.state().core.workbench.view(view).unwrap().kind,
        ViewKind::Media
    );
    app.harness.state_mut().core.save(id);
    app.harness.state_mut().core.save_as(id);
    app.harness
        .state_mut()
        .core
        .start_save(id, path.clone(), true, None, true);
    common::settle(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    common::run(&mut app, "media.zoomIn", json!({}));
    assert_eq!(
        app.harness.state().core.workbench.view(view).unwrap().zoom,
        Some(125.0)
    );
    common::run(&mut app, "media.fit", json!({}));
    assert_eq!(
        app.harness.state().core.workbench.view(view).unwrap().zoom,
        None
    );
    common::run(&mut app, "view.splitRight", json!({}));
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .views
            .of_document(id)
            .count(),
        2
    );
}
// spec 42: "Media settings expose playback defaults"
#[test]
fn media_settings_and_automatic_pause() {
    let (mut app, _dir, _path) = opened();
    let id = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    let core = &mut app.harness.state_mut().core;
    for key in [
        "media.autoplay",
        "media.pauseWhenHidden",
        "media.pauseOnFocusLoss",
        "media.resumeOnReturn",
    ] {
        assert!(core.settings.bool(key, None));
    }
    assert_eq!(core.settings.int("media.defaultVolume", None), 100);
    assert!(
        notoir::settings_tab::CATEGORIES.contains(&notoir_core::settings::SettingCategory::Media)
    );
    core.media
        .as_mut()
        .unwrap()
        .sessions
        .get_mut(&id)
        .unwrap()
        .playing = true;
    let ctx = egui::Context::default();
    core.tick_media(&ctx, false);
    assert!(!core.media.as_ref().unwrap().sessions[&id].playing);
    core.tick_media(&ctx, true);
    assert!(core.media.as_ref().unwrap().sessions[&id].playing);
    core.media_action(&notoir_core::command::CommandInvocation::new(
        "media.playPause",
        notoir_core::command::InvocationSource::Toolbar,
    ));
    core.tick_media(&ctx, false);
    core.tick_media(&ctx, true);
    assert!(!core.media.as_ref().unwrap().sessions[&id].playing);
}
// spec 42: "Closing the final media tab releases the session"
#[test]
fn final_close_releases_runtime() {
    let (mut app, _dir, _path) = opened();
    common::run(&mut app, "file.close", json!({}));
    app.harness.run_steps(2);
    assert!(
        app.harness
            .state()
            .core
            .workbench
            .active_document()
            .is_none()
    );
    assert!(app.harness.state().core.media.is_none());
}

#[test]
fn media_render_snapshots() {
    use egui_kittest::kittest::Queryable;
    for theme in notoir::theme::ThemeId::ALL {
        let (mut app, _dir, _path) = opened();
        let id = app
            .harness
            .state()
            .core
            .workbench
            .active_document()
            .unwrap();
        // Reuse the decoded picture as a deterministic paused-video poster;
        // this checks player controls without opening a physical audio device.
        app.harness
            .state_mut()
            .core
            .workbench
            .document_mut(id)
            .unwrap()
            .buffer = DocumentContent::Media(MediaKind::Video);
        app.harness.state_mut().core.theme_override = Some(theme);
        for width in [1280.0, 480.0] {
            app.harness.set_size(egui::vec2(width, 700.0));
            app.harness.run_steps(3);
            assert!(app.harness.query_by_label("Zoom in").is_some());
            assert!(app.harness.query_by_label("Play").is_some());
            let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/ui-snapshots");
            std::fs::create_dir_all(&output).unwrap();
            if let Ok(image) = app.harness.render() {
                image
                    .save(output.join(format!("media-{}-{}.png", theme.id(), width as u32)))
                    .unwrap();
            }
        }
    }
}

#[test]
fn media_session_restores_kind_zoom_and_paused_playback() {
    let mut app = common::start(|_| {});
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/media/video.webm");
    app.harness
        .state_mut()
        .core
        .open_path(&path, Default::default());
    common::settle(&mut app);
    common::run(&mut app, "media.actualSize", json!({}));
    let data = app.harness.state().core.capture_session();
    common::run(&mut app, "file.close", json!({}));
    app.harness.run_steps(2);
    app.harness.state_mut().core.apply_session(data);
    common::settle(&mut app);
    app.harness.run_steps(2);
    let core = &app.harness.state().core;
    let view = core
        .workbench
        .view(core.workbench.active_view().unwrap())
        .unwrap();
    assert_eq!(view.kind, ViewKind::Media);
    assert_eq!(view.zoom, Some(100.0));
    assert!(!core.media.as_ref().unwrap().sessions[&view.document_id].playing);
}
