//! Crash suite (docs/specs/33 §Reliability): runs the real binary with a
//! temporary NOTOIR_HOME, scripts it through the test command channel
//! (`NOTOIR_TEST_COMMANDS=1` + `--command`), kills it with SIGKILL and
//! relaunches. Needs a display, so it is ignored by default:
//! `cargo test -p notoir --test crash -- --ignored`.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(30);

struct App {
    home: PathBuf,
    child: Child,
    snapshots: Cell<u64>,
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
    // Exercise recovery without first-run UI or background update downloads.
    let settings = home.join("config/settings.toml");
    if !settings.exists() {
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "[update]\nmode = \"off\"\n").unwrap();
    }
    let child = command(home).args(args).spawn().expect("spawn notoir");
    let app = App {
        home: home.to_path_buf(),
        child,
        snapshots: Cell::new(0),
    };
    assert!(
        wait(TIMEOUT, || log(home)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|entry| {
                entry["fields"]["message"] == "test editor ready"
                    && entry["fields"]["pid"].as_u64() == Some(u64::from(app.child.id()))
            })),
        "editor did not render its first frame\n{}",
        log(home)
    );
    // The startup log precedes GPU/window creation. A reply from the actual
    // editor proves it has run a frame; the PID rejects stale relaunch state.
    wait_state(&app, "editor ready and session loaded", |state| {
        state["pid"].as_u64() == Some(u64::from(app.child.id()))
            && state["session_restoring"] == false
    });
    app
}

/// Sends commands to the running instance through a forwarded launch.
fn send(app: &App, id: &str, args: &str) {
    let mut child = command(&app.home)
        .args(["--command", id, "--args", args])
        .spawn()
        .expect("forward");
    let mut status = None;
    if !wait(TIMEOUT, || {
        status = child.try_wait().expect("forward status");
        status.is_some()
    }) {
        let _ = child.kill();
        let _ = child.wait();
        panic!("forwarded command {id} did not exit\n{}", log(&app.home));
    }
    assert!(status.unwrap().success(), "forwarded command {id} failed");
}

/// Recovery content and tab layout are separate asynchronous writes. Kill
/// only once the tab's owning window is recorded, so relaunch can find it.
fn wait_persisted_tab(app: &App, scratchpad: bool) {
    assert!(
        wait(TIMEOUT, || {
            let manifest = notoir_storage::state_db::StateDb::open(&app.home.join("data/state.db"))
                .ok()
                .and_then(|(db, _)| {
                    db.load_session(notoir::application::MANIFEST_KEY)
                        .ok()
                        .flatten()
                })
                .and_then(|json| {
                    serde_json::from_str::<notoir::application::ApplicationSession>(&json).ok()
                });
            manifest.is_some_and(|session| {
                session.windows.iter().any(|window| {
                    window.session.groups.iter().any(|group| {
                        group.tabs.iter().any(|tab| {
                            if scratchpad {
                                matches!(tab.source, notoir::session::TabSource::Scratchpad { .. })
                            } else {
                                matches!(tab.source, notoir::session::TabSource::File { .. })
                            }
                        })
                    })
                })
            })
        }),
        "tab ownership was not persisted\n{}",
        log(&app.home)
    );
}

fn snapshot(app: &App) -> Value {
    let sequence = app.snapshots.get();
    app.snapshots.set(sequence + 1);
    let output = app
        .home
        .join(format!("test-state-{}-{sequence}.json", app.child.id()));
    send(
        app,
        "test.ui",
        &json!({"op": "snapshot", "output": output}).to_string(),
    );
    let mut state = None;
    assert!(
        wait(TIMEOUT, || {
            state = std::fs::read(&output)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
            state.is_some()
        }),
        "editor did not answer snapshot request\n{}",
        log(&app.home)
    );
    state.unwrap()
}

fn wait_state(app: &App, description: &str, mut ready: impl FnMut(&Value) -> bool) {
    let mut state = Value::Null;
    assert!(
        wait(TIMEOUT, || {
            state = snapshot(app);
            ready(&state)
        }),
        "timed out waiting for {description}\n{state:#}\n{}",
        log(&app.home)
    );
}

fn active_text(state: &Value) -> Option<&str> {
    state["documents"]
        .as_array()?
        .iter()
        .find(|doc| doc["id"] == state["active_document"])?["text"]
        .as_str()
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

fn wait(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
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
    wait_state(&app, "file open before editing", |state| {
        active_text(state) == Some("original")
    });
    wait_persisted_tab(&app, false);
    send(&app, "editor.type", r#"{"text":"edited "}"#);
    wait_state(&app, "edit applied", |state| {
        active_text(state) == Some("edited original")
    });
    // The 1.5 s debounce writes the snapshot without any explicit flush.
    assert!(
        wait(TIMEOUT, || recovery_texts(&home)
            .iter()
            .any(|t| t == "edited original")),
        "snapshot written"
    );
    let home = kill(app);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");

    let app = launch(&home, &[]);
    wait_state(&app, "crash recovery offered", |state| {
        state["recovery_entries"].as_u64().unwrap_or(0) > 0
    });
    assert!(log(&home).contains("unsaved documents from a crashed session"));
    send(&app, "recovery.restoreAll", "{}");
    wait_state(&app, "recovered text before saving", |state| {
        active_text(state) == Some("edited original")
    });
    send(&app, "file.saveAll", "{}");
    assert!(wait(TIMEOUT, || std::fs::read_to_string(&file).unwrap()
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
    wait_state(&app, "Scratchpad created", |state| {
        active_text(state) == Some("")
    });
    wait_persisted_tab(&app, true);
    send(&app, "editor.type", r#"{"text":"phone 555"}"#);
    assert!(
        wait(TIMEOUT, || {
            std::fs::read_dir(home.join("data/scratchpads"))
                .unwrap()
                .flatten()
                .any(|entry| {
                    std::fs::read_to_string(entry.path()).is_ok_and(|text| text == "phone 555")
                })
        }),
        "Scratchpad was not persisted\n{}",
        log(&home)
    );
    let home = kill(app);
    let scratch = std::fs::read_dir(home.join("data").join("scratchpads"))
        .unwrap()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect::<Vec<_>>();
    assert!(scratch.iter().any(|t| t == "phone 555"), "{scratch:?}");
    let app = launch(&home, &[]);
    wait_state(&app, "Scratchpad restored", |state| {
        active_text(state) == Some("phone 555")
    });
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
    wait_state(&app, "Untitled document created", |state| {
        active_text(state) == Some("")
    });
    send(&app, "editor.type", r#"{"text":"safe"}"#);
    wait_state(&app, "edit applied", |state| {
        active_text(state) == Some("safe")
    });
    send(&app, "recovery.flush", "{}");
    let ok = wait(TIMEOUT, || {
        recovery_texts(&home).iter().any(|t| t == "safe")
    });
    assert!(ok, "{:?}\n{}", recovery_texts(&home), log(&home));
}
