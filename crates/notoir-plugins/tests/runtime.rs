//! The plugin VM against a stand-in for the UI thread: sandbox, API
//! requests, permission errors, time and memory limits.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use notoir_plugins::{
    ApiError, ErrorKind, FromPlugin, Permissions, PluginRuntime, RuntimeConfig, ToPlugin,
};
use serde_json::{Value, json};

fn plugin(dir: &Path, main: &str) -> RuntimeConfig {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("main.lua"), main).unwrap();
    RuntimeConfig {
        id: "test".into(),
        dir: dir.to_path_buf(),
        main: "main.lua".into(),
        permissions: Permissions::default(),
        language: "en".into(),
        memory_limit_mb: 16,
        command_budget: Duration::from_millis(300),
        event_budget: Duration::from_millis(100),
    }
}

/// Answers requests like the UI would: `notify` and `editor.active` work,
/// everything else is denied. Returns every event seen until `until`.
fn drive(rx: &Receiver<FromPlugin>, until: impl Fn(&FromPlugin) -> bool) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let Ok(e) = rx.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        let done = until(&e);
        match e {
            FromPlugin::Request(r) => {
                seen.push(format!("request {} {}", r.name, r.args));
                let reply = match r.name.as_str() {
                    "notify" | "commands.declared" | "events.subscribe" => Ok(Value::Null),
                    "editor.active" => Ok(json!(7)),
                    "editor.text" => Ok(json!("hello world")),
                    _ => Err(ApiError::new(
                        "permission_denied",
                        format!("{} is not allowed", r.name),
                    )),
                };
                r.respond(reply);
            }
            FromPlugin::Error {
                message,
                kind,
                during,
                ..
            } => seen.push(format!("error {kind:?} {during}: {message}")),
            FromPlugin::Log { level, message, .. } => seen.push(format!("log {level} {message}")),
            FromPlugin::Activated { .. } => seen.push("activated".into()),
            FromPlugin::CommandDone { command, .. } => seen.push(format!("done {command}")),
            FromPlugin::Exited { .. } => seen.push("exited".into()),
        }
        if done {
            break;
        }
    }
    seen
}

fn start(cfg: RuntimeConfig) -> (PluginRuntime, Receiver<FromPlugin>) {
    let (tx, rx) = channel();
    (PluginRuntime::start(cfg, tx, Arc::new(|| {})), rx)
}

#[test]
fn commands_run_with_a_lazy_editor_context() {
    let d = tempfile::tempdir().unwrap();
    let cfg = plugin(
        d.path(),
        r#"
        local util = require("lib.util")
        notoir.commands.register({ id = "count", run = function(ctx)
            local n = util.words(ctx.editor:text())
            notoir.notify(n .. " words")
        end })
        "#,
    );
    std::fs::create_dir_all(d.path().join("lib")).unwrap();
    std::fs::write(
        d.path().join("lib/util.lua"),
        "return { words = function(s) local n = 0; for _ in s:gmatch('%S+') do n = n + 1 end; return n end }",
    )
    .unwrap();
    let (rt, rx) = start(cfg);
    drive(&rx, |e| matches!(e, FromPlugin::Activated { .. }));
    rt.send(ToPlugin::Command {
        command: "count".into(),
        args: Value::Null,
    });
    let seen = drive(&rx, |e| matches!(e, FromPlugin::CommandDone { .. }));
    assert!(
        seen.contains(&r#"request notify {"level":"info","message":"2 words"}"#.to_owned()),
        "{seen:?}"
    );
}

#[test]
fn sandbox_has_no_io_os_debug_or_package() {
    let d = tempfile::tempdir().unwrap();
    let cfg = plugin(
        d.path(),
        r#"
        local missing = {}
        for _, name in ipairs({ "io", "debug", "package", "dofile", "loadfile" }) do
            if _G[name] ~= nil then table.insert(missing, name) end
        end
        if os.execute ~= nil or os.remove ~= nil or os.getenv ~= nil then table.insert(missing, "os.*") end
        assert(os.time() > 0)
        assert(load(string.dump(function() end)) == nil, "binary chunks load")
        assert(not pcall(require, "../escape"))
        print("leaks: " .. table.concat(missing, ","))
        "#,
    );
    let (_rt, rx) = start(cfg);
    let seen = drive(&rx, |e| {
        matches!(e, FromPlugin::Activated { .. } | FromPlugin::Error { .. })
    });
    assert!(seen.contains(&"log info leaks: ".to_owned()), "{seen:?}");
}

#[test]
fn denied_calls_raise_errors_in_lua() {
    let d = tempfile::tempdir().unwrap();
    let cfg = plugin(
        d.path(),
        r#"
        local ok, err = pcall(notoir.http.get, "http://example.com/")
        print(tostring(ok) .. " " .. tostring(err))
        local ok2, err2 = pcall(notoir.fs.read, "/etc/passwd")
        print(tostring(ok2) .. " " .. tostring(err2))
        "#,
    );
    let (_rt, rx) = start(cfg);
    let seen = drive(&rx, |e| matches!(e, FromPlugin::Activated { .. }));
    let logs: Vec<&String> = seen.iter().filter(|s| s.starts_with("log")).collect();
    assert!(
        logs[0].contains("false") && logs[0].contains("permission_denied"),
        "{logs:?}"
    );
    assert!(
        logs[1].contains("false") && logs[1].contains("permission_denied"),
        "{logs:?}"
    );
}

#[test]
fn infinite_loops_hit_the_time_limit() {
    let d = tempfile::tempdir().unwrap();
    let cfg = plugin(
        d.path(),
        r#"
        notoir.commands.register({ id = "spin", run = function() while true do end end })
        notoir.commands.register({ id = "stubborn", run = function()
            while true do pcall(function() while true do end end) end
        end })
        "#,
    );
    let (rt, rx) = start(cfg);
    drive(&rx, |e| matches!(e, FromPlugin::Activated { .. }));
    let t0 = Instant::now();
    rt.send(ToPlugin::Command {
        command: "spin".into(),
        args: Value::Null,
    });
    let seen = drive(&rx, |e| matches!(e, FromPlugin::Error { .. }));
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(
        seen.iter()
            .any(|s| s.starts_with("error Limit command spin") && s.contains("time limit")),
        "{seen:?}"
    );
    // A loop that catches the error cannot keep going either.
    let t0 = Instant::now();
    rt.send(ToPlugin::Command {
        command: "stubborn".into(),
        args: Value::Null,
    });
    let seen = drive(&rx, |e| matches!(e, FromPlugin::Error { .. }));
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(
        seen.iter()
            .any(|s| s.starts_with("error Limit command stubborn")),
        "{seen:?}"
    );
}

#[test]
fn memory_limit_is_enforced() {
    let d = tempfile::tempdir().unwrap();
    let cfg = plugin(
        d.path(),
        r#"
        notoir.commands.register({ id = "hog", run = function()
            local t = {}
            for i = 1, 1e9 do t[i] = string.rep("x", 1024) .. i end
        end })
        "#,
    );
    let mut cfg = cfg;
    cfg.command_budget = Duration::from_secs(20);
    let (rt, rx) = start(cfg);
    drive(&rx, |e| matches!(e, FromPlugin::Activated { .. }));
    rt.send(ToPlugin::Command {
        command: "hog".into(),
        args: Value::Null,
    });
    let seen = drive(&rx, |e| matches!(e, FromPlugin::Error { .. }));
    assert!(
        seen.iter()
            .any(|s| s.starts_with("error Limit") && s.contains("memory")),
        "{seen:?}"
    );
    let _ = ErrorKind::Script;
}

#[test]
fn script_errors_are_reported_with_location() {
    let d = tempfile::tempdir().unwrap();
    let cfg = plugin(
        d.path(),
        "notoir.events.on('on_file_save', function(ev) error('boom ' .. ev.path) end)",
    );
    let (rt, rx) = start(cfg);
    drive(&rx, |e| matches!(e, FromPlugin::Activated { .. }));
    rt.send(ToPlugin::Event {
        name: "on_file_save".into(),
        payload: json!({ "path": "a.md" }),
    });
    let seen = drive(&rx, |e| matches!(e, FromPlugin::Error { .. }));
    let err = seen.iter().find(|s| s.starts_with("error")).unwrap();
    assert!(
        err.starts_with("error Script event on_file_save")
            && err.contains("main.lua:1")
            && err.contains("boom a.md"),
        "{err}"
    );
}
