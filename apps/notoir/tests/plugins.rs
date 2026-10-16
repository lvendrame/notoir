//! Lua plugins in the app (specs 30, 05, 06, 32): approval, contributions
//! before activation, permissions, limits, errors and disabling, undoable
//! edits, events.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Event;
use egui_kittest::kittest::Queryable;
use notoir::plugins::PluginStatus;
use notoir_core::command::CommandId;
use notoir_platform::AppPaths;
use notoir_plugins::Manifest;
use notoir_storage::state_db::{PluginApprovalRow, StateDb};
use serde_json::json;

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

const WORD_COUNT_TOML: &str = r#"
[plugin]
id = "word-count"
name = "Word Count"
version = "1.0.0"
api = "1"

[permissions]
documents_read = true

[contributes]
commands = [{ id = "count", title = "%commands.count%" }]
keybindings = [{ command = "count", key = "Mod+Alt+W", when = "editorFocus" }]
menus = [{ command = "count", location = "plugins" }]
activation = ["onCommand:count"]
"#;

const WORD_COUNT_LUA: &str = r#"
notoir.commands.register({ id = "count", run = function(ctx)
    local n = 0
    for _ in ctx.editor:text():gmatch("%S+") do n = n + 1 end
    notoir.notify(notoir.i18n.t("words", { n = n }))
end })
"#;

/// Writes a plugin folder.
fn write_plugin(dir: &Path, toml: &str, lua: &str) {
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(dir.join("plugin.toml"), toml).unwrap();
    std::fs::write(dir.join("main.lua"), lua).unwrap();
    std::fs::write(
        dir.join("locales/en.json"),
        r#"{ "commands": { "count": "Count Words" }, "words": "{{n}} words" }"#,
    )
    .unwrap();
}

/// Records an approval as if the user had installed the plugin.
fn approve(home: &Path, toml: &str) {
    let m = Manifest::parse(toml).unwrap();
    let paths = AppPaths::under(home);
    paths.ensure_dirs().unwrap();
    let (db, _) = StateDb::open(&paths.data.join("state.db")).unwrap();
    db.set_plugin_approval(&PluginApprovalRow {
        plugin_id: m.plugin.id.clone(),
        version: m.plugin.version.clone(),
        permissions_hash: m.permissions.hash(),
        approved_at: 1,
        enabled: true,
    })
    .unwrap();
}

fn plugins_dir(home: &Path) -> PathBuf {
    AppPaths::under(home).plugins
}

/// A home with an installed plugin (approved unless `approved` is false),
/// a workspace with notes.md, and optional settings.
fn start_with(
    toml: &str,
    lua: &str,
    approved: bool,
    settings: &str,
) -> (tempfile::TempDir, PathBuf, common::TestApp) {
    let home = tempfile::tempdir().unwrap();
    let id = Manifest::parse(toml)
        .map(|m| m.plugin.id)
        .unwrap_or_else(|_| "broken".into());
    write_plugin(&plugins_dir(home.path()).join(&id), toml, lua);
    if approved {
        approve(home.path(), toml);
    }
    let paths = AppPaths::under(home.path());
    paths.ensure_dirs().unwrap();
    std::fs::write(paths.user_settings_file(), settings).unwrap();
    let ws = tempfile::tempdir().unwrap();
    let root = ws.path().canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "one two three\n").unwrap();
    let open = vec![(root.clone(), None), (root.join("notes.md"), None)];
    let mut app = common::start_in(home, move |o| o.open_paths = open);
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        core.plugins.scanned && core.workbench.active_document().is_some()
    }));
    (ws, root, app)
}

fn status(app: &common::TestApp, id: &str) -> Option<PluginStatus> {
    app.harness
        .state()
        .core
        .plugins
        .entry(id)
        .map(|e| e.status.clone())
}

fn notifications(app: &common::TestApp) -> Vec<String> {
    app.harness
        .state()
        .core
        .notifications
        .history()
        .map(notoir::shell::overlays::message_text)
        .collect()
}

fn output(app: &common::TestApp) -> Vec<String> {
    app.harness
        .state()
        .core
        .plugins
        .output
        .iter()
        .map(|(p, l, m)| format!("{p} {l} {m}"))
        .collect()
}

fn run(app: &mut common::TestApp, id: &str) {
    let inv = notoir_core::command::CommandInvocation::new(
        id.to_owned(),
        notoir_core::command::InvocationSource::Palette,
    );
    app.harness.state_mut().core.enqueue(inv);
    app.harness.run_steps(3);
}

// spec 30: "Command registered from manifest"
#[test]
fn command_registered_from_manifest() {
    let (_ws, _root, mut app) = start_with(WORD_COUNT_TOML, WORD_COUNT_LUA, true, "");
    let core = &app.harness.state().core;
    let id = CommandId::from("plugin.word-count.count".to_owned());
    let spec = core
        .commands
        .get(&id)
        .expect("registered from the manifest");
    assert_eq!(
        notoir::i18n::t_in(notoir::i18n::Language::En, &spec.title_key),
        "Count Words"
    );
    assert_eq!(status(&app, "word-count"), Some(PluginStatus::Enabled));
    // Listed in the Command Palette before activation.
    common::run(&mut app, "workbench.commandPalette", json!({}));
    app.harness
        .query_all_by_label_contains("Plugins: Count Words")
        .next()
        .expect("palette entry");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    run(&mut app, "plugin.word-count.count");
    assert!(wait_for(&mut app, |a| notifications(a)
        .iter()
        .any(|n| n == "Word Count: 3 words")));
    assert_eq!(status(&app, "word-count"), Some(PluginStatus::Active));
    // The keybinding and the Plugins menu entry are registered too.
    let core = &app.harness.state().core;
    assert!(core.plugins.bindings.iter().any(|b| b.command == id));
    assert_eq!(core.plugins.menu_items[0].title, "Count Words");
}

// spec 30: "Manually copied plugin is disabled"
#[test]
fn manually_copied_plugin_is_disabled() {
    let toml = WORD_COUNT_TOML.replace("[\"onCommand:count\"]", "[\"onStartup\"]");
    let lua = "notoir.notify('ran')";
    let (_ws, _root, mut app) = start_with(&toml, lua, false, "");
    assert_eq!(
        status(&app, "word-count"),
        Some(PluginStatus::NeedsReview { update: false })
    );
    common::settle(&mut app);
    let n = notifications(&app);
    assert!(n.iter().any(|t| t.contains("needs review")), "{n:?}");
    assert!(!n.iter().any(|t| t.contains("ran")), "code ran: {n:?}");
    let core = &app.harness.state().core;
    assert!(
        core.commands
            .get(&CommandId::from("plugin.word-count.count".to_owned()))
            .is_none()
    );
    // Review → Enable runs it.
    common::run(
        &mut app,
        "plugins.review",
        json!({ "plugin": "word-count" }),
    );
    app.harness
        .query_all_by_label_contains("Read open documents")
        .next()
        .expect("permission list");
    // The centred modal settles its size over a few frames.
    app.harness.run_steps(3);
    app.harness.get_by_label("Enable").click();
    app.harness.run_steps(3);
    assert_eq!(status(&app, "word-count"), Some(PluginStatus::Enabled));
}

// spec 30: "Update adding permissions requires re-approval"
#[test]
fn update_adding_permissions_requires_reapproval() {
    let (_ws, _root, mut app) = start_with(WORD_COUNT_TOML, WORD_COUNT_LUA, true, "");
    let v2 = tempfile::tempdir().unwrap();
    let toml2 = WORD_COUNT_TOML.replace("1.0.0", "1.1.0").replace(
        "documents_read = true",
        "documents_read = true\nnetwork = true",
    );
    write_plugin(v2.path(), &toml2, WORD_COUNT_LUA);
    app.harness
        .state_mut()
        .core
        .plugins_install_from(v2.path().to_path_buf());
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .review
        .is_some()));
    app.harness.run_steps(2);
    app.harness
        .query_all_by_label_contains("update of “Word Count” to 1.1.0")
        .next()
        .expect("update prompt");
    app.harness
        .query_all_by_label_contains("✓  Internet access")
        .next()
        .expect("new permission shown");
    // Not installed until approved.
    let installed = plugins_dir(app.home.path()).join("word-count/plugin.toml");
    assert!(
        std::fs::read_to_string(&installed)
            .unwrap()
            .contains("1.0.0")
    );
    app.harness.get_by_label("Install").click();
    assert!(wait_for(&mut app, |a| {
        a.harness
            .state()
            .core
            .plugins
            .entry("word-count")
            .and_then(|e| e.manifest.as_ref())
            .is_some_and(|m| m.plugin.version == "1.1.0")
    }));
    assert_eq!(status(&app, "word-count"), Some(PluginStatus::Enabled));
}

const PROBE_TOML: &str = r#"
[plugin]
id = "probe"
name = "Probe"
version = "1.0.0"
api = "1"

[permissions]
workspace_read = true
documents_read = true
documents_write = true

[contributes]
commands = [{ id = "go", title = "Probe" }, { id = "upper", title = "Upper" }, { id = "spin", title = "Spin" }]
activation = ["onCommand:go", "onCommand:upper", "onCommand:spin"]
"#;

const PROBE_LUA: &str = r#"
local function try(name, f, ...)
    local ok, err = pcall(f, ...)
    notoir.log.info(name .. " " .. tostring(ok) .. " " .. tostring(err))
end
notoir.commands.register({ id = "go", run = function()
    try("http", notoir.http.get, "http://127.0.0.1:" .. PORT .. "/")
    try("passwd", notoir.fs.read, "/etc/passwd")
    try("inside", notoir.fs.read, "notes.md")
    try("clipboard", notoir.clipboard.get)
    try("process", notoir.process.run, "echo", { "hi" })
end })
notoir.commands.register({ id = "upper", run = function(ctx)
    local sel = ctx.editor:selection()
    ctx.editor:replace(sel, string.upper(sel.text))
end })
notoir.commands.register({ id = "spin", run = function() while true do end end })
"#;

fn log_line<'a>(lines: &'a [String], name: &str) -> &'a str {
    lines
        .iter()
        .find(|l| l.starts_with(&format!("probe info {name} ")))
        .map_or("", String::as_str)
}

// spec 30: "Plugin without network permission cannot use http"
// spec 30: "Filesystem access outside workspace denied"
// spec 32: "Unrequested capability is denied without prompt"
#[test]
fn unrequested_capabilities_are_denied() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let lua = format!("PORT = {port}\n{PROBE_LUA}");
    let (_ws, _root, mut app) = start_with(PROBE_TOML, &lua, true, "");
    run(&mut app, "plugin.probe.go");
    assert!(wait_for(&mut app, |a| output(a)
        .iter()
        .any(|l| l.starts_with("probe info process"))));
    let lines = output(&app);
    let http = log_line(&lines, "http");
    assert!(
        http.contains("false") && http.contains("permission_denied"),
        "{lines:?}"
    );
    assert!(listener.accept().is_err(), "no network request is made");
    let passwd = log_line(&lines, "passwd");
    assert!(
        passwd.contains("false") && passwd.contains("permission_denied"),
        "{lines:?}"
    );
    assert!(
        log_line(&lines, "inside").contains("true one two three"),
        "{lines:?}"
    );
    let clip = log_line(&lines, "clipboard");
    assert!(
        clip.contains("false") && clip.contains("permission_denied"),
        "{lines:?}"
    );
    assert!(
        log_line(&lines, "process").contains("permission_denied"),
        "{lines:?}"
    );
    // No prompt was shown for any of them.
    let core = &app.harness.state().core;
    assert!(core.plugins.process_prompts.is_empty() && core.mcp.prompts.is_empty());
}

// spec 30: "Plugin edits are undoable"
#[test]
fn plugin_edits_are_undoable() {
    let lua = format!("PORT = 1\n{PROBE_LUA}");
    let (_ws, _root, mut app) = start_with(PROBE_TOML, &lua, true, "");
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    let _ = core.workbench.set_selection(
        view,
        notoir_editor::Selection::single(notoir_editor::selection::Range::new(4, 7)),
    );
    run(&mut app, "plugin.probe.upper");
    let text = |a: &common::TestApp| {
        let core = &a.harness.state().core;
        let doc = core.workbench.active_document().unwrap();
        core.workbench.document(doc).unwrap().buffer.text()
    };
    assert!(
        wait_for(&mut app, |a| text(a) == "one TWO three\n"),
        "{}",
        text(&app)
    );
    common::run(&mut app, "edit.undo", json!({}));
    assert_eq!(text(&app), "one two three\n");
}

// spec 30: "Infinite loop is stopped"
#[test]
fn infinite_loop_is_stopped() {
    let lua = format!("PORT = 1\n{PROBE_LUA}");
    let (_ws, _root, mut app) =
        start_with(PROBE_TOML, &lua, true, "plugins.commandTimeoutMs = 300\n");
    run(&mut app, "plugin.probe.spin");
    // The UI keeps rendering frames while the plugin spins.
    let t0 = Instant::now();
    app.harness.run_steps(5);
    assert!(t0.elapsed() < Duration::from_secs(2));
    assert!(wait_for(&mut app, |a| status(a, "probe")
        == Some(PluginStatus::Disabled)));
    let n = notifications(&app);
    assert!(
        n.iter()
            .any(|t| t == "Plugin “Probe” encountered an error and was disabled."),
        "{n:?}"
    );
    assert!(
        output(&app)
            .iter()
            .any(|l| l.contains("time limit exceeded"))
    );
}

const SAVER_TOML: &str = r#"
[plugin]
id = "saver"
name = "Saver"
version = "1.0.0"
api = "1"

[contributes]
activation = ["onStartup"]
"#;

// spec 30: "Repeated errors disable plugin"
#[test]
fn repeated_errors_disable_plugin() {
    let lua =
        "notoir.events.on('on_file_save', function(ev) error('cannot handle ' .. ev.title) end)";
    let (_ws, _root, mut app) = start_with(SAVER_TOML, lua, true, "");
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .entry("saver")
        .is_some_and(|e| e.subscribed.contains("on_file_save"))));
    for i in 0..3 {
        app.harness.event(Event::Text(format!("{i}")));
        app.harness.run_steps(2);
        common::run(&mut app, "file.save", json!({}));
        let errors = i + 1;
        assert!(wait_for(&mut app, |a| output(a)
            .iter()
            .filter(|l| l.contains("cannot handle notes.md"))
            .count()
            >= errors));
    }
    assert!(wait_for(&mut app, |a| status(a, "saver")
        == Some(PluginStatus::Disabled)));
    let core = &app.harness.state().core;
    let n = core
        .notifications
        .history()
        .find(|n| notoir::shell::overlays::message_text(n).contains("was disabled"))
        .expect("disabled notification");
    let actions: Vec<&str> = n.actions.iter().map(|a| a.label_key.as_str()).collect();
    assert_eq!(actions, ["plugins:viewError", "plugins:reenable"]);
    // Re-enable works.
    common::run(&mut app, "plugins.enable", json!({ "plugin": "saver" }));
    assert_eq!(status(&app, "saver"), Some(PluginStatus::Enabled));
}

// spec 06: "Change events are coalesced for plugins"
#[test]
fn change_events_are_coalesced_for_plugins() {
    let lua = "notoir.events.on('on_file_change', function(ev) notoir.log.info('change ' .. ev.title) end)";
    let (_ws, _root, mut app) = start_with(SAVER_TOML, lua, true, "");
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .entry("saver")
        .is_some_and(|e| e.subscribed.contains("on_file_change"))));
    // 20 characters within 100 ms.
    for c in "abcdefghijklmnopqrst".chars() {
        app.harness.event(Event::Text(c.to_string()));
        app.harness.step();
    }
    std::thread::sleep(Duration::from_millis(400));
    common::settle(&mut app);
    std::thread::sleep(Duration::from_millis(200));
    common::settle(&mut app);
    let changes = output(&app)
        .iter()
        .filter(|l| l.contains("change notes.md"))
        .count();
    assert_eq!(changes, 1, "{:?}", output(&app));
}

// spec 05: "Invalid plugin when-expression is rejected"
#[test]
fn invalid_plugin_when_expression_is_rejected() {
    let toml = WORD_COUNT_TOML.replace("when = \"editorFocus\"", "when = \"editorFocus &&\"");
    let (_ws, _root, app) = start_with(&toml, WORD_COUNT_LUA, true, "");
    let core = &app.harness.state().core;
    assert!(core.plugins.bindings.is_empty());
    let lines = output(&app);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("word-count error") && l.contains("editorFocus &&")),
        "{lines:?}"
    );
    // The command itself is still there.
    assert!(
        core.commands
            .get(&CommandId::from("plugin.word-count.count".to_owned()))
            .is_some()
    );
}

#[test]
fn malformed_manifest_is_invalid() {
    let (_ws, _root, app) = start_with("[plugin\nid = 1", "", false, "");
    match status(&app, "broken") {
        Some(PluginStatus::Invalid(e)) => assert!(e.contains("line"), "{e}"),
        s => panic!("{s:?}"),
    }
    assert!(output(&app).iter().any(|l| l.contains("Invalid plugin")));
}
