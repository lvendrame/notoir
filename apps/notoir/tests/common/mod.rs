//! Shared UI-test setup: every test runs with its own temporary NOTOIR_HOME.
#![allow(dead_code)]

use egui_kittest::Harness;
use notoir::app::NotoirApp;
use notoir::core::StartOptions;
use notoir::i18n::Language;
use notoir::theme::ThemeId;
use notoir_core::keymap::Platform;
use notoir_platform::AppPaths;

pub struct TestApp {
    pub harness: Harness<'static, NotoirApp>,
    pub home: tempfile::TempDir,
    /// The active UI language is process-wide: UI tests run one at a time.
    _serial: std::sync::MutexGuard<'static, ()>,
}

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Options for a fresh install in `home` (Linux conventions, English unless set).
pub fn options(home: &std::path::Path) -> StartOptions {
    let mut opts = StartOptions::new(AppPaths::under(home));
    opts.platform = Platform::Linux;
    opts.os_locale = Some("en-US".into());
    opts.wait_for_fonts = true;
    opts
}

pub fn start_in(home: tempfile::TempDir, configure: impl FnOnce(&mut StartOptions)) -> TestApp {
    let serial = serial();
    let mut opts = options(home.path());
    configure(&mut opts);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| NotoirApp::with_options(&cc.egui_ctx, opts));
    harness.run_steps(3);
    TestApp {
        harness,
        home,
        _serial: serial,
    }
}

pub fn start(configure: impl FnOnce(&mut StartOptions)) -> TestApp {
    start_in(tempfile::tempdir().unwrap(), configure)
}

pub fn with_language(lang: Language, theme: ThemeId) -> impl FnOnce(&mut StartOptions) {
    move |o| {
        o.language_override = Some(lang);
        o.theme_override = Some(theme);
    }
}

/// Runs a command as if chosen from the menu, then renders a few frames.
pub fn run(app: &mut TestApp, id: &'static str, args: serde_json::Value) {
    let inv = notoir_core::command::CommandInvocation::new(
        id,
        notoir_core::command::InvocationSource::Menu,
    )
    .with_args(args);
    app.harness.state_mut().core.enqueue(inv);
    app.harness.run_steps(3);
}

/// Stops the app (keeping its home) so another can start on the same data.
pub fn restart(app: TestApp, configure: impl FnOnce(&mut StartOptions)) -> TestApp {
    app.harness.state().core.flush_io();
    let TestApp {
        harness,
        home,
        _serial,
    } = app;
    drop(harness);
    drop(_serial);
    start_in(home, configure)
}
