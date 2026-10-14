//! Notoir — a fast native text editor and workspace.

use std::sync::mpsc::channel;
use std::sync::{Arc, OnceLock};

use notoir::{app, cli, icons, logging};
use notoir_platform::AppPaths;
use notoir_platform::single_instance::{self, Claim, InstanceRequest};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let cli = cli::parse(&args, &cwd);
    if cli.version {
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

    // Single instance: hand the command line to a running window (spec 31).
    let (tx, rx) = channel::<InstanceRequest>();
    let repaint: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let mut instance_rx = None;
    if !cli.new_window {
        let request = InstanceRequest {
            cwd: cwd.clone(),
            args: args.clone(),
        };
        let waker = repaint.clone();
        match single_instance::claim(&single_instance::socket_name(), &request, move |req| {
            let _ = tx.send(req);
            if let Some(ctx) = waker.get() {
                ctx.request_repaint();
            }
        }) {
            Ok(Claim::Forwarded) => {
                tracing::info!("forwarded to the running Notoir");
                return Ok(());
            }
            Ok(Claim::Primary) => instance_rx = Some(rx),
            Err(e) => tracing::warn!(error = %e, "single-instance socket unavailable"),
        }
    }

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
    let launch = app::Launch {
        open_paths: cli.paths,
        instance_rx,
    };
    let result = eframe::run_native(
        "Notoir",
        options,
        Box::new(move |cc| {
            let _ = repaint.set(cc.egui_ctx.clone());
            Ok(Box::new(app::NotoirApp::new(cc, launch)))
        }),
    );
    tracing::info!("exiting Notoir");
    result
}
