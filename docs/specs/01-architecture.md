# 01 — Architecture

## Purpose

Define the Rust workspace layout, crate responsibilities and dependency rules, application state ownership, the
mutation model, the async/background worker model, dependency choices, and the architectural decision log. Every
other spec assumes this structure.

## Workspace Layout

```text
notoir/
├── Cargo.toml                 # [workspace], shared deps, lints, profiles
├── rust-toolchain.toml        # stable + clippy + rustfmt
├── CLAUDE.md
├── README.md
├── apps/
│   └── notoir/                # eframe binary + UI (lib + main.rs)
│       ├── src/
│       │   ├── main.rs        # CLI args, logging init, eframe::run_native
│       │   ├── app.rs         # NotoirApp: eframe::App impl, frame loop
│       │   ├── shell/         # menu bar, toolbar, status bar, panels host, empty state
│       │   ├── editor_widget/ # text view, gutter, minimap painters, input handling
│       │   ├── panels/        # explorer, search, outline, scm, preview, json tree, mcp, plugins, output
│       │   ├── dialogs/       # modal-free prompts, palette, quick open
│       │   ├── commands/      # built-in command handlers (UI-facing glue)
│       │   ├── i18n.rs        # embedded locales, t!() macro
│       │   ├── icons.rs       # lucide icons → textures
│       │   ├── theme.rs       # tokens → egui::Visuals / editor palette
│       │   └── workers.rs     # job dispatch + result routing
│       ├── examples/gen_icons.rs
│       └── tests/             # egui_kittest UI tests + render snapshots
├── crates/
│   ├── notoir-core/
│   ├── notoir-editor/
│   ├── notoir-storage/
│   ├── notoir-workspace/
│   ├── notoir-platform/
│   ├── notoir-git/
│   ├── notoir-mcp/
│   └── notoir-plugins/
├── assets/
│   ├── icons/{lucide,custom,filetypes}/
│   ├── fonts/{ui,editor}/
│   ├── locales/<lng>/<ns>.json
│   ├── themes/*.toml          # built-in themes, same format as user themes
│   ├── syntaxes/              # extra .sublime-syntax files (TOML, TypeScript, …)
│   └── app-icon/
├── plugins/examples/          # sample Lua plugins (word-count, …)
└── docs/{specs,plan,design}/
```

## Crates

| Crate | Responsibility | May depend on | Must not depend on |
|---|---|---|---|
| `notoir-core` | Domain state & rules: `DocumentRegistry` (ids, metadata, dirty/revision tracking over `notoir-editor` buffers), `EditorView`, `EditorGroup`, `LayoutTree`, `PanelLayout`, `CommandRegistry`, `AppEvent`, settings schema + resolution, permission model types, workspace model, navigation history. | `notoir-editor`, serde, thiserror | egui, tokio, std::fs, network |
| `notoir-editor` | Text engine: rope buffer (`ropey`), transactions, undo/redo, selections/multi-cursor, line index, search/replace, folding ranges, outline extraction, syntax highlight state, Markdown/JSON structural parse with source spans, diff model. | ropey, regex, syntect (no IO features), pulldown-cmark, similar, unicode-segmentation | egui, tokio, std::fs |
| `notoir-storage` | User/workspace settings TOML IO, keybindings TOML, SQLite state DB (`rusqlite`) + migrations, recovery store (files), recent lists, Scratchpad store, atomic file writes. | notoir-core, rusqlite, toml, notoir-platform | egui |
| `notoir-workspace` | Directory scanning (`ignore`), explorer tree model updates, file watching (`notify`), workspace text search (`grep-*`), fuzzy file index (`nucleo-matcher`), file encoding detection & load. | notoir-core, notoir-editor, notoir-platform | egui |
| `notoir-platform` | App directories, file/folder dialogs (`rfd`), reveal in file manager, open URL (`open`), move to trash (`trash`), system font discovery (`fontdb`), single-instance lock, OS locale (`sys-locale`). | — | egui (dialogs return plain data) |
| `notoir-git` | Repository detection, status, diff, blobs, branches, commit via `git2`; fetch/pull/push/clone via system `git` CLI with git2 fallback. Pure request/response API; runs on workers. | notoir-core, git2 | egui |
| `notoir-mcp` | MCP server (`rmcp`), tool schemas, transport (Streamable HTTP on loopback), stdio bridge subcommand, client registry. Converts tool calls into `McpRequest`s sent to the UI thread. | notoir-core, rmcp, tokio, axum | egui |
| `notoir-plugins` | Lua host (`mlua`, Lua 5.4 vendored), manifest parsing, plugin lifecycle, API surface, permission gate. Runs plugin VMs on a host thread; talks to the UI thread through messages. | notoir-core, mlua, toml | egui |
| `apps/notoir` | eframe shell, all egui code, editor widget, command handler glue, worker orchestration. | all crates | — |

Rule: domain logic lives in `notoir-core` / `notoir-editor` and is unit-tested without a window. The UI crate is a
consumer of domain state and a producer of commands; it is never the authoritative business-logic layer.

## Application State Ownership

The UI thread owns all mutable application state in a single `AppCore` value, held by `NotoirApp`:

```rust
pub struct AppCore {
    pub documents: DocumentRegistry,      // Document id → Document (buffer + metadata)
    pub views: ViewRegistry,              // EditorView id → EditorView
    pub layout: LayoutTree,               // editor groups + splits
    pub panels: PanelLayout,              // sidebars, bottom panel, visibility, sizes
    pub workspace: Option<WorkspaceState>,
    pub settings: ResolvedSettings,       // user ⊕ workspace ⊕ defaults
    pub commands: CommandRegistry,
    pub keymap: Keymap,
    pub nav_history: NavigationHistory,
    pub notifications: NotificationCenter,
    pub permissions: PermissionStore,     // MCP + plugin grants (cached; persisted via storage)
    pub events: EventOutbox,              // typed events emitted during this frame
}
```

`NotoirApp` additionally owns UI-only state (`UiState`: open popups, palette query, drag-in-progress, hover, focus
target, per-panel scroll offsets) that never leaves the app crate.

Rules:

- `AppCore` is not wrapped in `Arc<Mutex<_>>`. Only the UI thread mutates it.
- Background workers receive **immutable snapshots** (`ropey::Rope` clones are O(1) and share structure; settings are
  `Arc<ResolvedSettings>`) and return typed results via channels.
- MCP tool calls and plugin API calls that need state are converted into messages handled on the UI thread through the
  same command handlers the UI uses (spec 05). No external subsystem holds a reference into `AppCore`.
- `AppCore` is split into focused services with their own methods; command handlers compose them. No god-object
  methods that "do everything".

## Mutation Model

```text
input source (keyboard / menu / toolbar / palette / Lua / MCP / worker result)
        │
        ▼
CommandInvocation { id, args, source }      ← enqueued on CommandQueue
        │  (UI thread, once per frame, FIFO)
        ▼
permission gate (source-dependent, spec 32)
        │
        ▼
command handler  ──►  domain services (DocumentRegistry, LayoutTree, …)
        │                    │
        │                    └─► EventOutbox (typed AppEvents)
        ▼
CommandOutcome (Ok / NeedsConfirmation / Err(UserError))
        │
        ▼
end of frame: drain EventOutbox → subscribers (session store, plugins, MCP notifications, UI caches)
```

Direct text editing in the editor widget is the one hot path that does not go through string-keyed commands for every
keystroke: the widget produces `EditOp`s applied through `DocumentRegistry::apply_transaction`, which is the same
function `document.apply_edits` (MCP) and `editor:replace` (Lua) ultimately call. Dirty tracking, undo, events and
recovery scheduling therefore behave identically regardless of origin.

## Async / Background Worker Model

- A single Tokio multi-thread runtime (2–4 worker threads, named `notoir-rt-*`) is created at startup on its own threads
  and lives for the process. A separate blocking pool (`spawn_blocking`) handles disk and libgit2 work.
- The UI thread never `.await`s. It submits `Job`s via `Workers::spawn(job)`; each job carries a `JobId`, a
  `CancellationToken`, and a `generation` number for its key (e.g. "workspace search", "preview for doc 7").
- Results return as `JobResult` over a `crossbeam_channel` (or `std::sync::mpsc`); the worker calls
  `egui::Context::request_repaint()` after sending. The UI drains the channel at the start of each frame.
- Stale results (generation older than the latest for the same key) are dropped. Starting a new job for a key cancels the
  previous one.
- Long jobs report progress (`JobProgress`) shown in the status bar activity indicator, never as a blocking modal.

| Work | Where |
|---|---|
| Workspace scanning, file indexing | Tokio blocking pool (`ignore` walker, parallel) |
| Workspace search | Blocking pool, streams partial results in batches |
| File load > 1 MiB, encoding detection | Blocking pool; tab shows loading state |
| File save | Blocking pool; doc marked "saving"; result updates saved revision (spec 09) |
| Recovery snapshots, autosave | Blocking pool, debounced from UI |
| Git local ops | Blocking pool (`git2` is sync) |
| Git network ops | Child process `git` with async stdout/stderr streaming |
| File watching | `notify` thread → debouncer → channel |
| MCP server I/O | Tokio tasks; requests → UI thread via channel, responses via oneshot |
| Plugin VMs | Dedicated plugin host thread per plugin (spec 30) |
| Markdown preview parse, JSON tree parse, syntax highlight of large docs | Blocking pool with debounce; small docs inline |

## Dependency Recommendations

Versions are pinned at scaffold time (M0) to the latest stable, mutually compatible releases, recorded in
`[workspace.dependencies]`. The reference project (`string_art_it_desktop`) currently pins `eframe/egui 0.36.2`,
`rmcp 3.5.1`, `tokio 1.53`, `resvg 0.48`, `rfd 0.17`, `directories 6`; start from those and bump.

| Concern | Crate | Notes |
|---|---|---|
| UI | `eframe`, `egui`, `egui_kittest` | glow backend, x11 + wayland features, `persistence` off (own persistence) |
| Text buffer | `ropey` | O(log n) edits, cheap clones for snapshots |
| Grapheme / width | `unicode-segmentation`, `unicode-width` | cursor movement by grapheme |
| Encodings | `encoding_rs`, `chardetng` | detect + decode + re-encode, BOM handling |
| Regex | `regex`, `fancy-regex` only if needed by syntect | search uses `regex` |
| Syntax | `syntect` (`default-fancy`, pure Rust) | + extra `.sublime-syntax` for TOML/TypeScript, compiled to a packdump at build time |
| Markdown | `pulldown-cmark` | with source offsets for scroll sync and outline |
| JSON | span-preserving parser (evaluate `jsonc-parser`; else own parser in `notoir-editor`) | ranges required for tree↔text sync |
| Diff | `similar` | line + inline char diff |
| Fuzzy matching | `nucleo-matcher` | Quick Open, palette |
| Dir walk / ignore | `ignore` | honours `.gitignore`, hidden files |
| Workspace search | `grep-searcher`, `grep-regex` | streaming results |
| File watching | `notify`, `notify-debouncer-full` | rename-aware debouncing |
| Settings / manifests | `serde`, `toml`, `toml_edit` (preserve user formatting/comments on write) | |
| State DB | `rusqlite` (`bundled`) | WAL mode; see spec 23 |
| Paths | `directories` | platform-standard dirs |
| Dialogs | `rfd` | native file/folder dialogs |
| Trash | `trash` | Delete in Explorer moves to OS trash |
| Open/reveal | `open` + per-OS reveal (spec 31) | |
| Fonts | `fontdb` | system font enumeration for editor font + CJK fallback |
| Icons | `resvg` / `tiny-skia` | rasterize lucide SVGs |
| Git | `git2` (vendored libgit2) + system `git` | see spec 28 |
| MCP | `rmcp` (server, streamable-http), `axum`, `tokio` | see spec 29 |
| Lua | `mlua` (`lua54`, `vendored`, `send`) | see spec 30 |
| Logging | `tracing`, `tracing-subscriber`, `tracing-appender` | rolling files, spec 35 |
| Errors | `thiserror` (libraries), `anyhow` (binary edges only) | |
| Channels | `crossbeam-channel` | |
| Locale | `sys-locale` | first-run language detection |
| Packaging | `cargo-packager` | spec 31 |

Lints: `unsafe_code = "forbid"` workspace-wide; clippy `all` as warnings, CI denies warnings.

## Decision Log

| # | Decision | Rationale | Alternatives rejected |
|---|---|---|---|
| AD-1 | Custom editor widget painted with egui primitives, not `egui::TextEdit` | Needs virtualization for large files, gutter, minimap, folding, multi-cursor, per-token colouring, bookmarks, IME control. `TextEdit` lays out the whole text every frame. | TextEdit + layouter (doesn't scale), embedding a native control (breaks cross-platform consistency) |
| AD-2 | Rope (`ropey`) buffer owned by `Document`; views hold only cursor/scroll state | Required by Document vs EditorView separation; O(1) snapshots for workers | Gap buffer (no cheap snapshots), `String` (O(n) edits) |
| AD-3 | Undo history is per Document, transactions record originating view's selections | Duplicate views must share one history; undo restores the selection in the view that undoes | Per-view history (diverges, confusing) |
| AD-4 | Own `LayoutTree` domain model for editor splits; rendering via an adapter | Persistable, testable, independent of docking crate; `egui_tiles`/`egui_dock` can be adopted behind the adapter later | Making `egui_dock::DockState` the source of truth |
| AD-5 | UI thread single owner of `AppCore`; message passing everywhere else | Avoids lock spaghetti and re-entrancy bugs | `Arc<Mutex<AppState>>` shared with MCP/plugins |
| AD-6 | Settings in TOML files; transient state in SQLite; recovery as plain files | TOML is user-editable and diff-friendly; SQLite gives atomic multi-record updates + migrations for session/recent/grants/activity; recovery files are independent of DB corruption and human-inspectable | Everything in SQLite; everything in JSON files |
| AD-7 | `git2` for local ops, system `git` CLI for network ops (git2 fallback) | System git honours the user's credential helpers, SSH config, proxies, LFS hooks; libgit2 auth is a known pain point | git2 only; `gix` (push support still maturing) |
| AD-8 | MCP over Streamable HTTP on 127.0.0.1 with per-client bearer tokens + a `notoir mcp-bridge` stdio subcommand | Works with HTTP-capable clients and stdio-only clients; tokens give real per-client identity for permissions | stdio only (one client, needs separate process), unauthenticated HTTP |
| AD-9 | One Lua VM per plugin on its own host thread, restricted stdlib | Plugin crash/timeout isolates to that plugin; UI never blocks on Lua | Shared VM on UI thread |
| AD-10 | "New Window" launches a new process; a single-instance socket routes "open file" from the OS to the most recent window | Simple and robust with eframe; each window has independent recovery session | egui multi-viewport for full windows (immature for this use) |
| AD-11 | In-window menu bar on all platforms for Phases 1–3; native macOS global menu evaluated as a later milestone (`muda`) | eframe has no native menu integration; consistency first | Native menus from day one |
| AD-12 | Zoom is global for all editor views by default (`editor.zoomScope = "global"`), optional per-view | Matches user expectation of "make text bigger"; per-view as power option | Per-view only |
| AD-13 | Line endings normalized to `\n` in the buffer; original style remembered and restored on save | Simplifies every algorithm; style is a document property shown in the status bar | Keeping raw `\r\n` in rope |

## Major Technical Risks

| Risk | Mitigation |
|---|---|
| Custom editor widget: IME, accessibility (AccessKit), bidi text, performance | Dedicated milestone (M4) with kittest tests; per-line galley cache keyed by (line content hash, wrap width, font, zoom); only visible lines laid out; AccessKit text node for the visible region; bidi out of scope for v1 (documented). |
| Word wrap + folding + virtual scrolling coordinate maps | Single `DisplayMap` in `notoir-editor` (buffer line ↔ display row) with unit tests; wrap computed lazily per visible range with estimated heights elsewhere. |
| eframe/egui upgrade churn | Pin versions; isolate egui usage in `apps/notoir`; upgrade in dedicated commits. |
| Git authentication across OSes | System git for network ops; clear error surfacing (spec 28). |
| Lua sandbox escapes / runaway scripts | Restricted stdlib, memory limit, instruction-count hook, per-call timeouts, permission gate in Rust (spec 30). |
| MCP security (DNS rebinding, other local processes) | Loopback bind, Host/Origin allow-lists, bearer tokens, permission prompts, activity log (spec 29). |
| Data loss via save path or recovery bugs | Atomic writes, recovery built early (M8), crash tests in CI (spec 33). |
| Syntax highlighting cost on large files | Incremental per-line parse state cache; async beyond a size threshold; large-file mode disables it. |
| macOS packaging/signing/notarization | Structured release workflow from M11; signing optional until certificates exist. |

## Test Cases

```gherkin
Feature: Layer boundaries

  Scenario: Domain crates do not depend on UI or IO crates
    Given the workspace Cargo metadata
    When the dependency graph of notoir-core and notoir-editor is inspected
    Then neither depends on egui, eframe, tokio, rusqlite, notify, or git2

Feature: Worker results

  Scenario: Stale results are discarded
    Given a workspace search job with generation 1 is running
    When the user changes the query and generation 2 starts
    And generation 1 later delivers results
    Then the results of generation 1 are dropped
    And only generation 2 results are displayed

  Scenario: UI stays responsive during background work
    Given a workspace with 100,000 files is being scanned
    When the user types in an open document
    Then each frame completes without waiting on the scan
```

## Notes

- The layer-boundary test is implemented with `cargo metadata` in an integration test under `tests/` at workspace level.
- `AppCore` services expose `&mut self` methods only; there are no interior-mutability types in `notoir-core` except
  `Arc` for immutable shared data.
