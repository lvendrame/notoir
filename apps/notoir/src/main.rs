//! Notoir — a fast native text editor and workspace.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::sync::mpsc::channel;
use std::sync::{Arc, OnceLock};

use notoir::{app, application, cli, logging};
use notoir_platform::AppPaths;
use notoir_platform::single_instance::{self, Claim, InstanceRequest};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `notoir mcp-bridge --token <t> [--port <p>]`: MCP over stdio, proxied
    // to the running app (spec 29). Never opens a window; stdout is protocol.
    if args.first().map(String::as_str) == Some("mcp-bridge") {
        match notoir_mcp::bridge::parse_args(&args[1..]) {
            Ok((token, port)) => {
                let stdin = std::io::stdin();
                if let Err(e) = notoir_mcp::bridge::Bridge::new(&token, port)
                    .run(stdin.lock(), std::io::stdout())
                {
                    eprintln!("notoir mcp-bridge: {e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("notoir mcp-bridge: {e}");
                std::process::exit(2);
            }
        }
        return Ok(());
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    // Terminal output follows the OS language (settings are not read yet).
    let lang = notoir::i18n::Language::detect(notoir_platform::os_locale().as_deref(), None);
    let cli = match cli::parse(&args, &cwd) {
        Ok(cli) => cli,
        Err(e) => {
            notoir_platform::attach_parent_console();
            eprintln!("notoir: {}", e.message(lang));
            eprintln!("{}", notoir::i18n::t_in(lang, "cli:seeHelp"));
            std::process::exit(2);
        }
    };
    if cli.help || cli.version || cli.wait {
        notoir_platform::attach_parent_console();
    }
    if cli.help {
        print!("{}", cli::usage(lang));
        return Ok(());
    }
    if cli.version {
        println!("{}", cli::version_line());
        return Ok(());
    }
    // `--wait` (spec 38): this process only forwards and waits; the window
    // lives in the running instance, started here when there is none.
    if cli.wait {
        std::process::exit(wait_for_instance(&args, &cwd));
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
    let (tx, rx) = channel::<(InstanceRequest, Option<single_instance::Waiter>)>();
    let repaint: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let instance_rx;
    let accepting_launches = Arc::new(std::sync::atomic::AtomicBool::new(true));
    {
        let request = InstanceRequest {
            cwd: cwd.clone(),
            args: args.clone(),
        };
        let waker = repaint.clone();
        match single_instance::claim_with_gate(
            &single_instance::socket_name(),
            &request,
            accepting_launches.clone(),
            move |req, w| {
                let _ = tx.send((req, w));
                if let Some(ctx) = waker.get() {
                    ctx.request_repaint();
                }
            },
        ) {
            Ok(Claim::Forwarded) => {
                tracing::info!("forwarded to the running Notoir");
                return Ok(());
            }
            Ok(Claim::Primary) => instance_rx = Some(rx),
            Err(e) => return Err(eframe::Error::AppCreation(Box::new(e))),
        }
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        paths = ?paths.as_ref().ok(),
        "starting Notoir"
    );
    let viewport = egui::ViewportBuilder::default()
        .with_title("Notoir")
        .with_app_id(notoir_platform::paths::BUNDLE_ID)
        .with_visible(false)
        .with_inner_size([1.0, 1.0])
        .with_position([-10000.0, -10000.0])
        .with_decorations(false)
        .with_taskbar(false);
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
        ..Default::default()
    };
    let launch = app::Launch {
        restore_session: cli.paths.is_empty() && !cli.new_window,
        open_paths: cli.paths,
        instance_rx,
        accepting_launches: Some(accepting_launches),
    };
    let result = eframe::run_native(
        "Notoir",
        options,
        Box::new(move |cc| {
            let _ = repaint.set(cc.egui_ctx.clone());
            Ok(Box::new(application::ApplicationHost::new(cc, launch)))
        }),
    );
    tracing::info!("exiting Notoir");
    result
}

/// `--wait`: forwards to the running instance (starting one without
/// `--wait` if needed) and returns once the files are closed there.
fn wait_for_instance(args: &[String], cwd: &std::path::Path) -> i32 {
    if let Err(e) = AppPaths::resolve().and_then(|paths| paths.ensure_dirs()) {
        eprintln!("notoir: {e}");
        return 1;
    }
    let name = single_instance::socket_name();
    let mut request = InstanceRequest {
        cwd: cwd.to_path_buf(),
        args: args.to_vec(),
    };
    match single_instance::forward(&name, &request) {
        Ok(true) => return 0,
        Ok(false) => {}
        Err(e) => {
            eprintln!("notoir: {e}");
            return 1;
        }
    }
    // No instance: open the requested paths without restoring unrelated windows,
    // then attach the waiter to those already-open documents.
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("notoir: {e}");
            return 1;
        }
    };
    if let Err(e) = std::process::Command::new(exe)
        .args(without_flags(args, &["--wait", "-w"]))
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        eprintln!("notoir: {e}");
        return 1;
    }
    request.args = without_flags(&request.args, &["--new-window", "-n"]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(true) = single_instance::forward(&name, &request) {
            return 0;
        }
    }
    eprintln!("notoir: Notoir did not start");
    1
}

/// Remove launch-control switches while preserving literal paths after `--`.
fn without_flags(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut positional = false;
    args.iter()
        .filter(|arg| {
            if arg.as_str() == "--" {
                positional = true;
            }
            positional || !flags.contains(&arg.as_str())
        })
        .cloned()
        .collect()
}
