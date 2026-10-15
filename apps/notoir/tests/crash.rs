//! Crash suite (docs/specs/33 §Reliability): runs the real binary with a
//! temporary NOTOIR_HOME, scripts it through the test command channel
//! (`NOTOIR_TEST_COMMANDS=1` + `--command`), kills it with SIGKILL and
//! relaunches. Needs a display, so it is ignored by default:
//! `cargo test -p notoir --test crash -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct App {
    home: PathBuf,
    child: Child,
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_notoir"));
    c.env("NOTOIR_HOME", home)
        .env("NOTOIR_TEST_COMMANDS", "1")
        .env("NOTOIR_LOG", "info");
    c
}

fn launch(home: &Path, args: &[&str]) -> App {
    let child = command(home).args(args).spawn().expect("spawn notoir");
    let app = App {
        home: home.to_path_buf(),
        child,
    };
    assert!(
        wait(Duration::from_secs(20), || log(home)
            .contains("starting Notoir")),
        "app did not start"
    );
    // Let the first frames run (startup work, single-instance socket).
    std::thread::sleep(Duration::from_millis(1500));
    app
}

/// Sends commands to the running instance through a forwarded launch.
fn send(app: &App, id: &str, args: &str) {
    let status = command(&app.home)
        .args(["--command", id, "--args", args])
        .status()
        .expect("forward");
    assert!(status.success());
    std::thread::sleep(Duration::from_millis(400));
}

fn log(home: &Path) -> String {
    std::fs::read_dir(home.join("logs"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn wait(timeout: Duration, f: impl Fn() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn recovery_texts(home: &Path) -> Vec<String> {
    fn walk(p: &Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|x| x == "txt") {
                    out.push(std::fs::read_to_string(&path).unwrap_or_default());
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&home.join("data").join("recovery"), &mut out);
    out
}

fn kill(mut app: App) -> PathBuf {
    app.child.kill().expect("SIGKILL");
    let _ = app.child.wait();
    app.home.clone()
}

// Crash with unsaved files (spec 33 §Reliability).
#[test]
#[ignore = "spawns the real app (needs a display); run with --ignored"]
fn kill_with_unsaved_edits_then_restore() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let file = dir.path().join("a.md");
    std::fs::write(&file, "original").unwrap();
    let app = launch(&home, &[file.to_str().unwrap()]);
    send(&app, "editor.type", r#"{"text":"edited "}"#);
    // The 1.5 s debounce writes the snapshot without any explicit flush.
    assert!(
        wait(Duration::from_secs(6), || recovery_texts(&home)
            .iter()
            .any(|t| t == "edited original")),
        "snapshot written"
    );
    let home = kill(app);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");

    let app = launch(&home, &[]);
    assert!(log(&home).contains("unsaved documents from a crashed session"));
    send(&app, "recovery.restoreAll", "{}");
    send(&app, "file.saveAll", "{}");
    assert!(wait(Duration::from_secs(5), || std::fs::read_to_string(
        &file
    )
    .unwrap()
        == "edited original"));
}

// Crash with Scratchpads.
#[test]
#[ignore = "spawns the real app (needs a display); run with --ignored"]
fn kill_with_scratchpad_then_relaunch() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let app = launch(&home, &[]);
    send(&app, "file.newScratchpad", "{}");
    send(&app, "editor.type", r#"{"text":"phone 555"}"#);
    std::thread::sleep(Duration::from_millis(800));
    let home = kill(app);
    let scratch = std::fs::read_dir(home.join("data").join("scratchpads"))
        .unwrap()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect::<Vec<_>>();
    assert!(scratch.iter().any(|t| t == "phone 555"), "{scratch:?}");
    let _app = launch(&home, &[]);
    assert!(!log(&home).contains("panicked"));
}

// Corrupt state.db: starts fresh, keeps the corrupt file, recovery still works.
#[test]
#[ignore = "spawns the real app (needs a display); run with --ignored"]
fn corrupt_state_db_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("data")).unwrap();
    std::fs::write(
        home.join("data").join("state.db"),
        b"this is not a database",
    )
    .unwrap();
    let app = launch(&home, &[]);
    let kept = std::fs::read_dir(home.join("data"))
        .unwrap()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("state.db.corrupt-")
        });
    assert!(kept);
    send(&app, "file.newUntitled", "{}");
    send(&app, "editor.type", r#"{"text":"safe"}"#);
    send(&app, "recovery.flush", "{}");
    let ok = wait(Duration::from_secs(5), || {
        recovery_texts(&home).iter().any(|t| t == "safe")
    });
    assert!(ok, "{:?}\n{}", recovery_texts(&home), log(&home));
}
