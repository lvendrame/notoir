//! One plugin VM on its own thread (docs/specs/30 §Runtime, AD-9): a
//! sandboxed Lua 5.4 state with memory and time limits. API calls become
//! [`PluginRequest`]s answered by the UI thread; the plugin thread blocks on
//! the reply, the UI thread never waits on Lua. File, network, process and
//! clipboard IO runs here, after the UI thread allowed it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Value as LuaValue, VmState};
use serde_json::{Value, json};

use crate::manifest::Permissions;

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// How long a blocked API call waits for the UI thread (permission prompts
/// last up to 120 s).
const REPLY_TIMEOUT: Duration = Duration::from_secs(150);
/// Process runs end after this long.
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
/// Instructions between budget checks.
const HOOK_EVERY: u32 = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// `permission_denied`, `invalid_args`, `not_supported`…
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

pub type ApiResult = Result<Value, ApiError>;

/// An API call waiting for the UI thread.
#[derive(Debug)]
pub struct PluginRequest {
    pub plugin: String,
    /// `notify`, `editor.text`, `fs.check`…
    pub name: String,
    pub args: Value,
    reply: Sender<ApiResult>,
}

impl PluginRequest {
    pub fn respond(self, r: ApiResult) {
        let _ = self.reply.send(r);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// A Lua error (counted; three within 60 s disable the plugin).
    Script,
    /// Time or memory limit: the plugin is disabled at once.
    Limit,
}

/// What a plugin thread reports to the UI thread.
#[derive(Debug)]
pub enum FromPlugin {
    Request(PluginRequest),
    /// `main.lua` ran.
    Activated {
        plugin: String,
    },
    CommandDone {
        plugin: String,
        command: String,
    },
    Log {
        plugin: String,
        level: String,
        message: String,
    },
    Error {
        plugin: String,
        /// What was running: `activation`, `command count`, `event on_file_save`.
        during: String,
        message: String,
        kind: ErrorKind,
    },
    /// The thread ended (shutdown or a runaway script).
    Exited {
        plugin: String,
    },
}

#[derive(Debug)]
pub enum ToPlugin {
    Command { command: String, args: Value },
    Event { name: String, payload: Value },
    Shutdown,
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub id: String,
    pub dir: PathBuf,
    pub main: String,
    pub permissions: Permissions,
    /// UI language code (`pt-BR`) for `notoir.i18n.t`.
    pub language: String,
    pub memory_limit_mb: usize,
    pub command_budget: Duration,
    pub event_budget: Duration,
}

/// A running plugin VM. Dropping it asks the thread to stop.
pub struct PluginRuntime {
    pub id: String,
    tx: Sender<ToPlugin>,
}

impl PluginRuntime {
    /// Starts the thread and activates the plugin (runs `main`).
    pub fn start(cfg: RuntimeConfig, events: Sender<FromPlugin>, wake: Wake) -> PluginRuntime {
        let (tx, rx) = channel();
        let id = cfg.id.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("plugin-{id}"))
            .spawn(move || {
                let plugin = cfg.id.clone();
                let out = Out {
                    events: events.clone(),
                    wake: wake.clone(),
                    plugin: plugin.clone(),
                };
                // A runaway script is ended by unwinding out of the VM.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(cfg, rx, &out);
                }));
                out.send(FromPlugin::Exited { plugin });
            });
        if let Err(e) = spawned {
            tracing::error!(plugin = %id, error = %e, "could not start plugin thread");
        }
        PluginRuntime { id, tx }
    }

    pub fn send(&self, msg: ToPlugin) -> bool {
        self.tx.send(msg).is_ok()
    }
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        let _ = self.tx.send(ToPlugin::Shutdown);
    }
}

#[derive(Clone)]
struct Out {
    events: Sender<FromPlugin>,
    wake: Wake,
    plugin: String,
}

impl Out {
    fn send(&self, e: FromPlugin) {
        if self.events.send(e).is_ok() {
            (self.wake)();
        }
    }

    fn error(&self, during: &str, e: &mlua::Error, budget: &Budget) {
        let kind = if budget.exceeded || matches!(e, mlua::Error::MemoryError(_)) {
            ErrorKind::Limit
        } else {
            ErrorKind::Script
        };
        let message = match e {
            mlua::Error::MemoryError(_) => "memory limit exceeded".to_owned(),
            e => e.to_string(),
        };
        self.send(FromPlugin::Error {
            plugin: self.plugin.clone(),
            during: during.to_owned(),
            message,
            kind,
        });
    }
}

/// The time budget of the running call. Time spent waiting for the UI
/// thread does not count.
#[derive(Default)]
struct Budget {
    started: Option<Instant>,
    waited: Duration,
    limit: Duration,
    exceeded: bool,
}

impl Budget {
    fn start(&mut self, limit: Duration) {
        *self = Budget {
            started: Some(Instant::now()),
            waited: Duration::ZERO,
            limit,
            exceeded: false,
        };
    }

    fn stop(&mut self) {
        self.started = None;
    }

    fn over(&self) -> bool {
        self.started
            .is_some_and(|s| s.elapsed().saturating_sub(self.waited) > self.limit)
    }
}

fn lua_err(e: ApiError) -> mlua::Error {
    mlua::Error::RuntimeError(format!("{}: {}", e.code, e.message))
}

fn run(cfg: RuntimeConfig, rx: Receiver<ToPlugin>, out: &Out) {
    let budget = Rc::new(RefCell::new(Budget::default()));
    let lua = match sandbox(&cfg, out, budget.clone()) {
        Ok(lua) => lua,
        Err(e) => {
            out.error("activation", &e, &budget.borrow());
            return;
        }
    };
    // Activation: run main.
    budget.borrow_mut().start(cfg.command_budget);
    let main = cfg.dir.join(&cfg.main);
    let result = std::fs::read_to_string(&main)
        .map_err(mlua::Error::external)
        .and_then(|src| {
            lua.load(src)
                .set_name(format!("@{}", cfg.main))
                .set_mode(mlua::chunk::ChunkMode::Text)
                .exec()
        });
    budget.borrow_mut().stop();
    if let Err(e) = result {
        out.error("activation", &e, &budget.borrow());
        return;
    }
    out.send(FromPlugin::Activated {
        plugin: cfg.id.clone(),
    });
    while let Ok(msg) = rx.recv() {
        match msg {
            ToPlugin::Shutdown => break,
            ToPlugin::Command { command, args } => {
                budget.borrow_mut().start(cfg.command_budget);
                let r = call_global(&lua, "_run_command", (command.clone(), args));
                budget.borrow_mut().stop();
                match r {
                    Ok(()) => out.send(FromPlugin::CommandDone {
                        plugin: cfg.id.clone(),
                        command,
                    }),
                    Err(e) => out.error(&format!("command {command}"), &e, &budget.borrow()),
                }
            }
            ToPlugin::Event { name, payload } => {
                budget.borrow_mut().start(cfg.event_budget);
                let r = call_global(&lua, "_emit", (name.clone(), payload));
                budget.borrow_mut().stop();
                if let Err(e) = r {
                    out.error(&format!("event {name}"), &e, &budget.borrow());
                }
            }
        }
    }
}

/// Calls `notoir.<name>(a, json)`.
fn call_global(lua: &Lua, name: &str, (a, json): (String, Value)) -> mlua::Result<()> {
    let notoir: mlua::Table = lua.globals().get("notoir")?;
    let f: mlua::Function = notoir.get(name)?;
    let v = to_lua(lua, &json)?;
    f.call::<()>((a, v))
}

fn to_lua(lua: &Lua, v: &Value) -> mlua::Result<LuaValue> {
    lua.to_value_with(
        v,
        mlua::serde::ser::Options::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false),
    )
}

fn from_lua(lua: &Lua, v: LuaValue) -> mlua::Result<Value> {
    match v {
        LuaValue::Nil => Ok(Value::Null),
        v => lua.from_value(v),
    }
}

/// A Lua state with only the safe libraries and the `notoir` API.
fn sandbox(cfg: &RuntimeConfig, out: &Out, budget: Rc<RefCell<Budget>>) -> mlua::Result<Lua> {
    let libs = StdLib::STRING
        | StdLib::TABLE
        | StdLib::MATH
        | StdLib::UTF8
        | StdLib::COROUTINE
        | StdLib::OS;
    let lua = Lua::new_with(libs, LuaOptions::default())?;
    lua.set_memory_limit(cfg.memory_limit_mb.max(1) * 1024 * 1024)?;
    let g = lua.globals();
    // os: only time, clock and date.
    let os: mlua::Table = g.get("os")?;
    let safe_os = lua.create_table()?;
    for k in ["time", "clock", "date"] {
        safe_os.set(k, os.get::<LuaValue>(k)?)?;
    }
    g.set("os", safe_os)?;
    for k in ["dofile", "loadfile"] {
        g.set(k, LuaValue::Nil)?;
    }
    // load: text chunks only.
    let load: mlua::Function = g.get("load")?;
    let text_load = lua.create_function(
        move |_, (chunk, name, _mode, env): (LuaValue, LuaValue, LuaValue, LuaValue)| {
            load.call::<mlua::MultiValue>((chunk, name, "t", env))
        },
    )?;
    g.set("load", text_load)?;
    install_require(&lua, &cfg.dir)?;

    // Time budget. Once exceeded, every check raises again and the
    // error-catching functions re-raise it, so a script cannot keep going.
    {
        let budget = budget.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_EVERY),
            move |_, _| {
                let mut b = budget.borrow_mut();
                if b.exceeded || b.over() {
                    b.exceeded = true;
                    return Err(mlua::Error::RuntimeError("time limit exceeded".into()));
                }
                Ok(VmState::Continue)
            },
        )?;
    }
    let co: mlua::Table = g.get("coroutine")?;
    for (table, name) in [(g.clone(), "pcall"), (g.clone(), "xpcall"), (co, "resume")] {
        let inner: mlua::Function = table.get(name)?;
        let budget = budget.clone();
        let wrapped = lua.create_function(move |_, args: mlua::MultiValue| {
            let r = inner.call::<mlua::MultiValue>(args)?;
            if budget.borrow().exceeded {
                return Err(mlua::Error::RuntimeError("time limit exceeded".into()));
            }
            Ok(r)
        })?;
        table.set(name, wrapped)?;
    }

    let native = lua.create_table()?;
    // call(name, args): an API request answered by the UI thread.
    let requester = Requester {
        out: out.clone(),
        budget: budget.clone(),
    };
    {
        let r = requester.clone();
        native.set(
            "call",
            lua.create_function(move |lua, (name, args): (String, LuaValue)| {
                let args = from_lua(lua, args)?;
                let v = r.request(&name, args).map_err(lua_err)?;
                to_lua(lua, &v)
            })?,
        )?;
    }
    {
        let r = requester.clone();
        native.set(
            "fs",
            lua.create_function(
                move |lua, (op, path, text): (String, String, Option<String>)| {
                    let allowed = r
                        .request("fs.check", json!({ "op": op, "path": path }))
                        .map_err(lua_err)?;
                    let real = PathBuf::from(allowed["path"].as_str().unwrap_or_default());
                    fs_op(lua, &op, &real, text)
                },
            )?,
        )?;
    }
    {
        let r = requester.clone();
        native.set(
            "http_get",
            lua.create_function(move |lua, (url, _opts): (String, LuaValue)| {
                let host = url
                    .split("://")
                    .nth(1)
                    .and_then(|rest| rest.split(['/', ':', '?']).next())
                    .unwrap_or("")
                    .to_owned();
                r.request("http.check", json!({ "url": url, "host": host }))
                    .map_err(lua_err)?;
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .timeout_global(Some(Duration::from_secs(30)))
                    .build()
                    .into();
                let mut resp = agent
                    .get(&url)
                    .call()
                    .map_err(|e| mlua::Error::RuntimeError(format!("network_error: {e}")))?;
                let status = resp.status().as_u16();
                let body = resp
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| mlua::Error::RuntimeError(format!("network_error: {e}")))?;
                to_lua(lua, &json!({ "status": status, "body": body }))
            })?,
        )?;
    }
    {
        let r = requester.clone();
        native.set(
            "process_run",
            lua.create_function(
                move |lua, (cmd, args, opts): (String, Vec<String>, LuaValue)| {
                    let opts = from_lua(lua, opts)?;
                    let allowed = r
                        .request(
                            "process.check",
                            json!({ "command": cmd, "args": args, "cwd": opts.get("cwd") }),
                        )
                        .map_err(lua_err)?;
                    let cwd = allowed["cwd"].as_str().map(PathBuf::from);
                    let res = run_process(&cmd, &args, cwd.as_deref())
                        .map_err(|e| mlua::Error::RuntimeError(format!("process_error: {e}")))?;
                    to_lua(lua, &res)
                },
            )?,
        )?;
    }
    {
        let r = requester.clone();
        native.set(
            "clipboard",
            lua.create_function(move |lua, (op, text): (String, Option<String>)| {
                r.request("clipboard.check", json!({ "op": op }))
                    .map_err(lua_err)?;
                let mut cb = arboard::Clipboard::new()
                    .map_err(|e| mlua::Error::RuntimeError(format!("clipboard_error: {e}")))?;
                if op == "set" {
                    cb.set_text(text.unwrap_or_default())
                        .map_err(|e| mlua::Error::RuntimeError(format!("clipboard_error: {e}")))?;
                    Ok(LuaValue::Nil)
                } else {
                    let t = cb.get_text().unwrap_or_default();
                    Ok(LuaValue::String(lua.create_string(t)?))
                }
            })?,
        )?;
    }
    {
        let o = out.clone();
        native.set(
            "log",
            lua.create_function(move |_, (level, message): (String, String)| {
                o.send(FromPlugin::Log {
                    plugin: o.plugin.clone(),
                    level,
                    message,
                });
                Ok(())
            })?,
        )?;
    }
    {
        let table = Rc::new(load_locales(&cfg.dir, &cfg.language));
        native.set(
            "t",
            lua.create_function(move |lua, (key, args): (String, LuaValue)| {
                let mut s = table.get(&key).cloned().unwrap_or_else(|| key.clone());
                if let Value::Object(map) = from_lua(lua, args)? {
                    for (k, v) in map {
                        let v = match v {
                            Value::String(s) => s,
                            v => v.to_string(),
                        };
                        s = s.replace(&format!("{{{{{k}}}}}"), &v);
                    }
                }
                Ok(s)
            })?,
        )?;
    }
    g.set("__notoir", native)?;
    lua.load(include_str!("prelude.lua"))
        .set_name("@notoir")
        .exec()?;
    drop(g);
    Ok(lua)
}

#[derive(Clone)]
struct Requester {
    out: Out,
    budget: Rc<RefCell<Budget>>,
}

impl Requester {
    /// Sends a request and blocks until the UI thread answers; the wait
    /// does not count against the time budget.
    fn request(&self, name: &str, args: Value) -> ApiResult {
        let (tx, rx) = channel();
        self.out.send(FromPlugin::Request(PluginRequest {
            plugin: self.out.plugin.clone(),
            name: name.to_owned(),
            args,
            reply: tx,
        }));
        let t0 = Instant::now();
        let r = rx.recv_timeout(REPLY_TIMEOUT);
        self.budget.borrow_mut().waited += t0.elapsed();
        r.unwrap_or_else(|_| Err(ApiError::new("unavailable", "Notoir did not answer")))
    }
}

/// `require("a.b")`: `a/b.lua` inside the plugin directory only.
fn install_require(lua: &Lua, dir: &Path) -> mlua::Result<()> {
    let root = dir.canonicalize().map_err(mlua::Error::external)?;
    let cache = lua.create_table()?;
    let f = lua.create_function(move |lua, name: String| {
        if let Ok(v) = cache.get::<LuaValue>(name.as_str())
            && !v.is_nil()
        {
            return Ok(v);
        }
        if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err(mlua::Error::RuntimeError(format!(
                "module '{name}' not found"
            )));
        }
        let rel = format!("{}.lua", name.replace('.', "/"));
        let path = root.join(&rel);
        let real = path
            .canonicalize()
            .ok()
            .filter(|p| p.starts_with(&root))
            .ok_or_else(|| mlua::Error::RuntimeError(format!("module '{name}' not found")))?;
        let src = std::fs::read_to_string(&real).map_err(mlua::Error::external)?;
        let v: LuaValue = lua
            .load(src)
            .set_name(format!("@{rel}"))
            .set_mode(mlua::chunk::ChunkMode::Text)
            .call(name.clone())?;
        let v = if v.is_nil() {
            LuaValue::Boolean(true)
        } else {
            v
        };
        cache.set(name, v.clone())?;
        Ok(v)
    })?;
    lua.globals().set("require", f)
}

/// The plugin's `locales/<lang>.json` over `locales/en.json`, flattened.
pub fn load_locales(dir: &Path, language: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for lang in ["en", language] {
        let Ok(text) = std::fs::read_to_string(dir.join("locales").join(format!("{lang}.json")))
        else {
            continue;
        };
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            flatten("", &v, &mut out);
        }
    }
    out
}

fn flatten(prefix: &str, v: &Value, out: &mut HashMap<String, String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
        }
        _ => {}
    }
}

fn fs_op(lua: &Lua, op: &str, path: &Path, text: Option<String>) -> mlua::Result<LuaValue> {
    let io = |e: std::io::Error| mlua::Error::RuntimeError(format!("io_error: {e}"));
    match op {
        "read" => {
            let s = std::fs::read_to_string(path).map_err(io)?;
            Ok(LuaValue::String(lua.create_string(s)?))
        }
        "list" => {
            let mut names: Vec<Value> = std::fs::read_dir(path)
                .map_err(io)?
                .filter_map(Result::ok)
                .map(|e| {
                    json!({
                        "name": e.file_name().to_string_lossy(),
                        "dir": e.file_type().is_ok_and(|t| t.is_dir()),
                    })
                })
                .collect();
            names.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            to_lua(lua, &Value::Array(names))
        }
        "write" => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(io)?;
            }
            std::fs::write(path, text.unwrap_or_default()).map_err(io)?;
            Ok(LuaValue::Boolean(true))
        }
        other => Err(mlua::Error::RuntimeError(format!("unknown fs op {other}"))),
    }
}

fn run_process(cmd: &str, args: &[String], cwd: Option<&Path>) -> std::io::Result<Value> {
    use std::io::Read;
    let mut c = std::process::Command::new(cmd);
    c.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let mut child = c.spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if start.elapsed() > PROCESS_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(json!({
        "code": status.and_then(|s| s.code()),
        "stdout": out_t.join().unwrap_or_default(),
        "stderr": err_t.join().unwrap_or_default(),
        "timed_out": status.is_none(),
    }))
}
