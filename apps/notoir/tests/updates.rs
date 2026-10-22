//! Updates (spec 31): signed manifests from a local server, the offer,
//! download with checksum, and the installer hand-off at exit. Its own
//! test binary: it installs a global log subscriber to check warnings.

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use notoir_platform::update::{UpdateConfig, UpdateError, platform_key};
use serde_json::Value;
use sha2::Digest;

/// Everything logged by this test binary.
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = {
            let buf = buf.clone();
            move || LogWriter(buf.clone())
        };
        let _ = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .try_init();
        buf
    })
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serves fixed bodies by path on 127.0.0.1; returns the base URL.
fn serve(files: HashMap<String, Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body) = match files.get(&path) {
                Some(b) => ("200 OK", b.clone()),
                None => ("404 Not Found", Vec::new()),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    base
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// (minisign public key, minisign signature of `msg`) for a seed.
fn key_and_sign(seed: u8, msg: &[u8]) -> (String, String) {
    let sk = SigningKey::from_bytes(&[seed; 32]);
    let id = [seed; 8];
    let mut public = b"Ed".to_vec();
    public.extend(id);
    public.extend(sk.verifying_key().as_bytes());
    let sig = sk.sign(msg).to_bytes();
    let mut line = b"Ed".to_vec();
    line.extend(id);
    line.extend(sig);
    let trusted = "timestamp:0\tfile:update.json";
    let mut global = sig.to_vec();
    global.extend(trusted.as_bytes());
    let global = sk.sign(&global).to_bytes();
    (
        B64.encode(public),
        format!(
            "untrusted comment: test\n{}\ntrusted comment: {trusted}\n{}\n",
            B64.encode(line),
            B64.encode(global)
        ),
    )
}

fn manifest(base: &str, sha: &str) -> Vec<u8> {
    let art = format!(r#"{{"url":"{base}/Notoir-99.0.0.dmg","sha256":"{sha}"}}"#);
    format!(
        r#"{{"version":"99.0.0","artifacts":{{"macos":{art},"windows":{art},"linux-appimage":{art}}}}}"#
    )
    .into_bytes()
}

fn start(cfg: UpdateConfig, dialogs: &notoir::files::ScriptedDialogs) -> common::TestApp {
    let d = dialogs.clone();
    let mut app = common::start(move |o| {
        o.update = Some(cfg);
        o.dialogs = Some(Arc::new(d));
    });
    // Linux builds outside an AppImage have no updater (their package
    // manager updates them); test the AppImage flow there.
    app.harness.state_mut().core.updates.platform =
        Some(platform_key().unwrap_or("linux-appimage"));
    app
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

fn has_notice(app: &common::TestApp, key: &str) -> bool {
    app.harness
        .state()
        .core
        .notifications
        .history()
        .any(|n| n.message.key == key)
}

// spec 31: "Invalid signature is rejected"
#[test]
fn invalid_signature_is_rejected() {
    let logs = logs();
    let m = manifest("http://unused", &"00".repeat(32));
    let (key, _) = key_and_sign(1, &m);
    // Signed with another key.
    let (_, wrong) = key_and_sign(2, &m);
    let base = serve(HashMap::from([
        ("/update.json".to_owned(), m),
        ("/update.json.minisig".to_owned(), wrong.into_bytes()),
    ]));
    let dialogs = notoir::files::ScriptedDialogs::default();
    let mut app = start(
        UpdateConfig {
            manifest_url: format!("{base}/update.json"),
            public_key: key,
        },
        &dialogs,
    );
    common::run(&mut app, "app.checkForUpdates", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .updates
        .last_error
        .is_some()));
    let core = &app.harness.state().core;
    assert!(matches!(
        core.updates.last_error,
        Some(UpdateError::Signature(_))
    ));
    assert!(core.updates.offer.is_none(), "no update offered");
    assert!(!has_notice(&app, "notifications:updateAvailable"));
    let text = String::from_utf8_lossy(&logs.lock().unwrap()).into_owned();
    assert!(
        text.contains("WARN") && text.contains("update check failed"),
        "warning logged: {text}"
    );
}

#[test]
fn verified_update_is_offered_downloaded_and_handed_off_at_exit() {
    let installer = b"pretend installer".to_vec();
    let sha: String = sha2::Sha256::digest(&installer)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    // The server's address is only known once it runs: serve the manifest
    // through a second listener that points at the first.
    let files_base = serve(HashMap::from([(
        "/Notoir-99.0.0.dmg".to_owned(),
        installer.clone(),
    )]));
    let m = manifest(&files_base, &sha);
    let (key, sig) = key_and_sign(3, &m);
    let base = serve(HashMap::from([
        ("/update.json".to_owned(), m),
        ("/update.json.minisig".to_owned(), sig.into_bytes()),
    ]));
    let dialogs = notoir::files::ScriptedDialogs::default();
    let mut app = start(
        UpdateConfig {
            manifest_url: format!("{base}/update.json"),
            public_key: key,
        },
        &dialogs,
    );
    common::run(&mut app, "app.checkForUpdates", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .updates
        .offer
        .is_some()));
    assert_eq!(
        app.harness
            .state()
            .core
            .updates
            .offer
            .as_ref()
            .map(|o| o.version.as_str()),
        Some("99.0.0")
    );
    // Nothing is downloaded or installed without the user.
    assert!(app.harness.state().core.updates.ready.is_none());
    common::run(&mut app, "app.downloadUpdate", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .updates
        .ready
        .is_some()));
    let path = app.harness.state().core.updates.ready.clone().unwrap();
    assert!(path.starts_with(&app.harness.state().core.paths.cache));
    assert_eq!(std::fs::read(&path).unwrap(), installer);
    // Quitting without choosing Install on Quit installs nothing.
    common::run(&mut app, "app.installUpdateOnQuit", Value::Null);
    assert!(has_notice(&app, "notifications:updateWillInstall"));
    app.harness.state_mut().core.shutdown();
    assert_eq!(*dialogs.installed.lock().unwrap(), vec![path]);
}

#[test]
fn a_tampered_download_is_discarded() {
    let files_base = serve(HashMap::from([(
        "/Notoir-99.0.0.dmg".to_owned(),
        b"evil".to_vec(),
    )]));
    // The signed manifest names another checksum.
    let m = manifest(&files_base, &"ab".repeat(32));
    let (key, sig) = key_and_sign(4, &m);
    let base = serve(HashMap::from([
        ("/update.json".to_owned(), m),
        ("/update.json.minisig".to_owned(), sig.into_bytes()),
    ]));
    let dialogs = notoir::files::ScriptedDialogs::default();
    let mut app = start(
        UpdateConfig {
            manifest_url: format!("{base}/update.json"),
            public_key: key,
        },
        &dialogs,
    );
    common::run(&mut app, "app.checkForUpdates", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .updates
        .offer
        .is_some()));
    common::run(&mut app, "app.downloadUpdate", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .updates
        .last_error
        .is_some()));
    let core = &app.harness.state().core;
    assert!(core.updates.ready.is_none());
    let dir = core.paths.cache.join("update");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(leftovers.iter().all(|n| n == "last-check"), "{leftovers:?}");
    app.harness.state_mut().core.shutdown();
    assert!(dialogs.installed.lock().unwrap().is_empty());
}

// spec 31: "Quit and Install preserves unsaved documents when cancelled"
#[test]
fn cancelling_quit_keeps_verified_update_and_unsaved_text() {
    use notoir_core::settings::SettingValue;
    let dialogs = notoir::files::ScriptedDialogs::default();
    let d = dialogs.clone();
    let mut app = common::start(move |o| o.dialogs = Some(Arc::new(d)));
    common::run(&mut app, "file.newUntitled", Value::Null);
    common::run(
        &mut app,
        "editor.type",
        serde_json::json!({"text":"keep this"}),
    );
    let core = &mut app.harness.state_mut().core;
    core.set_user_setting("app.hotExit", Some(SettingValue::Bool(true)));
    let path = core.paths.cache.join("verified.dmg");
    core.updates.ready = Some(path.clone());
    core.updates.open = true;
    core.enqueue(notoir_core::command::CommandInvocation::new(
        "app.quitAndInstall",
        notoir_core::command::InvocationSource::Menu,
    ));
    core.run_queued(&core.context(false));
    assert!(core.updates.install_on_quit);
    assert!(!core.request_quit());
    core.answer_prompt(notoir::files::Answer::Cancel);
    assert!(!core.updates.install_on_quit);
    assert_eq!(core.updates.ready, Some(path));
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert_eq!(doc.buffer.text(), "keep this");
    assert!(doc.is_dirty());
    core.shutdown();
    assert!(dialogs.installed.lock().unwrap().is_empty());
}

#[test]
fn cancelling_save_as_cancels_update_shutdown() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newUntitled", Value::Null);
    common::run(
        &mut app,
        "editor.type",
        serde_json::json!({"text":"keep draft"}),
    );
    {
        let core = &mut app.harness.state_mut().core;
        core.updates.ready = Some(core.paths.cache.join("verified.dmg"));
        core.enqueue(notoir_core::command::CommandInvocation::new(
            "app.quitAndInstall",
            notoir_core::command::InvocationSource::Menu,
        ));
        core.run_queued(&core.context(false));
        assert!(!core.request_quit());
        core.answer_prompt(notoir::files::Answer::Primary);
    }
    common::settle(&mut app);
    let core = &app.harness.state().core;
    assert!(!core.updates.install_on_quit);
    assert!(core.updates.ready.is_some());
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert_eq!(doc.buffer.text(), "keep draft");
    assert!(doc.is_dirty());
}
