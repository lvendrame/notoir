//! The `notoir` command (spec 38): the real binary for help, version and
//! errors; the app for what a command line opens; `--wait` over the
//! single-instance socket.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use notoir::cli;
use notoir_platform::single_instance::{self, Claim, InstanceRequest};

fn notoir(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    let t0 = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_notoir"))
        .args(args)
        .env("NOTOIR_HOME", home.path())
        .env("LANG", "en_US.UTF-8")
        .output()
        .unwrap();
    // Exiting at once means no window was opened.
    assert!(t0.elapsed() < Duration::from_secs(10));
    out
}

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

#[test]
fn version_prints_the_product_and_version() {
    let out = notoir(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("Notoir {}", env!("CARGO_PKG_VERSION"))
    );
}

// spec 38: "Help prints usage without launching the app"
#[test]
fn help_prints_usage_without_launching_the_app() {
    let out = notoir(&["--help"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for part in ["--new-window", "--goto", "--wait", "--version", "notoir ["] {
        assert!(text.contains(part), "{part} missing:\n{text}");
    }
}

#[test]
fn invalid_options_fail_with_a_hint() {
    for args in [&["--frobnicate"][..], &["--goto"][..]] {
        let out = notoir(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains(args[0]) && err.contains("notoir --help"),
            "{err}"
        );
        assert!(out.stdout.is_empty());
    }
}

/// A project folder with notes.md and src/main.rs.
fn project() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "notes\n").unwrap();
    let main: String = (1..=20).map(|i| format!("line {i} of main\n")).collect();
    std::fs::write(root.join("src/main.rs"), main).unwrap();
    (d, root)
}

/// Starts the app as `notoir <args>` would from `cwd`.
fn launch(args: &[&str], cwd: &std::path::Path) -> common::TestApp {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let cli = cli::parse(&args, cwd).unwrap();
    common::start(move |o| {
        o.restore_session = cli.paths.is_empty() && !cli.new_window;
        o.open_paths = cli.paths;
    })
}

fn workspace_root(app: &common::TestApp) -> Option<PathBuf> {
    app.harness
        .state()
        .core
        .workspace
        .as_ref()
        .map(|w| w.root.clone())
}

fn active_path(app: &common::TestApp) -> Option<PathBuf> {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document()?;
    core.workbench
        .document(doc)?
        .path()
        .map(std::path::Path::to_path_buf)
}

// spec 38: "Dot opens the terminal directory as workspace"
#[test]
fn dot_opens_the_terminal_directory_as_workspace() {
    let (_d, root) = project();
    let mut app = launch(&["."], &root);
    assert!(wait_for(&mut app, |a| workspace_root(a).as_ref() == Some(&root)));
}

// spec 38: "Multiple paths open from the terminal directory"
#[test]
fn multiple_paths_open_from_the_terminal_directory() {
    let (_d, root) = project();
    let mut app = launch(&["notes.md", "src"], &root);
    assert!(wait_for(&mut app, |a| {
        workspace_root(a).as_ref() == Some(&root.join("src"))
            && a.harness
                .state()
                .core
                .workbench
                .documents
                .values()
                .any(|d| d.path() == Some(root.join("notes.md").as_path()))
    }));
}

// spec 38: "Goto opens a file at a line and column"
#[test]
fn goto_opens_a_file_at_a_line_and_column() {
    let (_d, root) = project();
    let mut app = launch(&["--goto", "src/main.rs:12:4"], &root);
    assert!(wait_for(&mut app, |a| active_path(a)
        == Some(root.join("src/main.rs"))));
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
    let head = view.selection.primary().head;
    let line = rope.char_to_line(head);
    assert_eq!((line + 1, head - rope.line_to_char(line) + 1), (12, 4));
}

// spec 38: "Bare command restores the prior session"
#[test]
fn bare_command_restores_the_prior_session() {
    let (_d, root) = project();
    let mut app = launch(&["."], &root);
    assert!(wait_for(&mut app, |a| workspace_root(a).is_some()));
    app.harness
        .state_mut()
        .core
        .open_path(&root.join("notes.md"), notoir::files::OpenAt::default());
    assert!(wait_for(&mut app, |a| active_path(a) == Some(root.join("notes.md"))));
    // `notoir` with no arguments: restore.
    let cli = cli::parse(&[], &root).unwrap();
    assert!(cli.paths.is_empty() && !cli.new_window);
    let mut app = common::restart(app, |o| o.restore_session = true);
    assert!(wait_for(&mut app, |a| {
        workspace_root(a).as_ref() == Some(&root) && active_path(a) == Some(root.join("notes.md"))
    }));
}

// spec 38: "Wait exits after opened tabs close"
#[test]
fn wait_exits_after_opened_tabs_close() {
    let (_d, root) = project();
    let mut app = common::start(|_| {});
    // The app is the primary instance on a private socket.
    let name = format!("notoir-cli-wait-{}.sock", std::process::id());
    let (tx, rx) = std::sync::mpsc::channel();
    let first = InstanceRequest {
        cwd: root.clone(),
        args: vec![],
    };
    let claimed = single_instance::claim(&name, &first, move |r, w| {
        let _ = tx.send((r, w));
    })
    .unwrap();
    assert!(matches!(claimed, Claim::Primary));
    // `notoir --wait notes.md` from another process.
    let request = InstanceRequest {
        cwd: root.clone(),
        args: vec!["--wait".into(), "notes.md".into()],
    };
    let n = name.clone();
    let terminal = std::thread::spawn(move || single_instance::forward(&n, &request).unwrap());
    let (req, waiter) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(waiter.is_some());
    app.harness
        .state_mut()
        .core
        .handle_instance_request(req, waiter);
    assert!(wait_for(&mut app, |a| active_path(a) == Some(root.join("notes.md"))));
    std::thread::sleep(Duration::from_millis(300));
    common::settle(&mut app);
    assert!(
        !terminal.is_finished(),
        "the terminal waits while the tab is open"
    );
    common::run(&mut app, "file.close", serde_json::json!({}));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !terminal.is_finished() {
        assert!(Instant::now() < deadline, "the terminal was not released");
        common::settle(&mut app);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(terminal.join().unwrap());
}

#[test]
fn real_mcp_bridge_preserves_piped_stdio() {
    use std::io::Write;
    use std::process::Stdio;
    use std::sync::{Arc, mpsc::channel};

    use notoir_core::permissions::ClientId;
    use notoir_mcp::auth::{generate_token, hash_token};
    use notoir_mcp::{ClientAuth, ClientRegistry, McpServer};

    let clients = ClientRegistry::default();
    let token = generate_token();
    clients.add(ClientAuth {
        id: ClientId("stdio-test".into()),
        token_hash: hash_token(&token),
        revoked: false,
    });
    let (tx, _events) = channel();
    let server = McpServer::start(0, clients, tx, Arc::new(|| {})).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_notoir"))
        .args([
            "mcp-bridge",
            "--token",
            &token,
            "--port",
            &server.port().to_string(),
        ])
        .env("NOTOIR_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                "protocolVersion":"2025-06-18", "capabilities":{},
                "clientInfo":{"name":"stdio-test","version":"1"}
            }
        })
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "notoir");
}

#[cfg(windows)]
#[test]
fn windows_executable_uses_gui_subsystem() {
    let exe = std::fs::read(env!("CARGO_BIN_EXE_notoir")).unwrap();
    assert_eq!(&exe[..2], b"MZ");
    let pe = u32::from_le_bytes(exe[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&exe[pe..pe + 4], b"PE\0\0");
    let optional_header = pe + 4 + 20;
    let subsystem = u16::from_le_bytes(
        exe[optional_header + 68..optional_header + 70]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        subsystem, 2,
        "Windows GUI subsystem must not allocate a console"
    );
}
