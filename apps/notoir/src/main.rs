//! Notoir — a fast native text editor and workspace.

use notoir::{app, icons, logging};
use notoir_platform::AppPaths;

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("notoir {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let paths = AppPaths::resolve().and_then(|p| p.ensure_dirs().map(|()| p));
    let _log_guard = match &paths {
        Ok(p) => logging::init(&p.logs),
        Err(e) => {
            eprintln!("notoir: {e}");
            None
        }
    };
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        paths = ?paths.as_ref().ok(),
        "starting Notoir"
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Notoir")
            .with_app_id(notoir_platform::paths::BUNDLE_ID)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            .with_icon(icons::app_icon()),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Notoir",
        options,
        Box::new(|cc| Ok(Box::new(app::NotoirApp::new(cc)))),
    );
    tracing::info!("exiting Notoir");
    result
}
