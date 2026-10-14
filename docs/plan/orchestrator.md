# Notoir Orchestrator

Milestones implement the specs in `docs/specs/` phase by phase (spec 00 §Phases). Status: `Todo` → `In progress` →
`Done`. Update notes when closing a milestone (decisions taken, deviations, follow-ups).

Per milestone: read the listed specs → define data models / commands / UI state → turn the specs' Gherkin scenarios
into tests → implement → `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test
--workspace` → CI green on Windows/macOS/Linux → refactor → update this table.

## Phase 1 — Core editor foundation

| # | Milestone | Specs | Status | Notes |
|---|---|---|---|---|
| M0 | Scaffold: workspace + crates skeleton, toolchain, lints, CI matrix (fmt/clippy/test/deny), `notoir-platform` paths + `NOTOIR_HOME`, tracing logs, lucide 1.48 icon loader, fonts (Manrope/JetBrains Mono), theme tokens → egui Visuals for 3 themes, i18n core (embedded JSON, `t!`, 14 locales, key-parity + literal lint), layer-boundary test, app icon | 01, 21, 22, 31, 33, 35 | Done | eframe/egui 0.36.2, toolchain 1.97. Bundle id `dev.notoir.Notoir` (ProjectDirs `dev`/`Notoir`/`Notoir`; macOS config nested in `config/` because config == data dir there). Only core/editor/platform crates created; others on demand. i18n: global active language + `t!`/`tn!`, CLDR plurals for the 14 languages, parity checked on base keys (plural suffixes stripped); namespaces `common`, `welcome`. Themes parse TOML with `{ base, alpha }` derivations; all three pass WCAG checks with spec 21 starting palette (tertiary text adjusted: Midnight `#7a7f8c`, Paper `#75776e`, Dracula `#8390bd`). 64 lucide icons (stroke 1.75). Empty state only (no commands yet). Font metrics/glyph checks use `skrifa` (already pulled in by epaint) — `ttf-parser` is flagged unmaintained (RUSTSEC-2026-0192). CI: fmt, deny, clippy+test × 3 OS. Follow-up (M3): CJK system-font scan (`fontdb::load_system_fonts`, ~0.9 s) runs on the UI thread at startup for CJK languages — move off-thread / defer to keep the 300 ms startup budget. |
| M1 | Domain core: `Document` (rope, encoding, EOL, language detection, large-file flags), `DocumentRegistry`, transactions/undo, `Selections` + change mapping, `EditorView`, `EditorGroup`, `LayoutTree` ops, `CommandRegistry` + when-expressions + keymap resolution, `EventOutbox`, settings schema + resolution | 02, 03, 05, 06, 11 (model parts), 20 (resolution) | Todo | Pure crates, unit + proptest only. |
| M2 | Storage: settings/keybindings TOML IO (toml_edit, watched), SQLite state DB + migrations + corruption handling, recent list, Scratchpad store, atomic write helper, recovery store format (write/scan/lock/restore model) | 08, 09 (format), 20, 23 | Todo | |
| M3 | App shell: eframe window, menu bar (data-driven), toolbar, status bar, panel system (primary/secondary/bottom, activity bar), empty state, notifications, theme switching, language switching, Help → About; design mockup `docs/design/notoir-mockup.html` | 17, 18, 19, 21, 22 | Todo | |
| M4 | Editor widget: virtualized rendering, DisplayMap (wrap), caret/selection, keyboard + mouse input, IME, clipboard, undo/redo wiring, gutter (line numbers lane), current line, whitespace, zoom, word wrap toggle, AccessKit node, large-file behaviour | 11, 12 (gutter), 14 | Todo | Highest-risk milestone; perf budgets from spec 33. |
| M5 | Files & tabs: Open File, New File, New Scratchpad, Save/Save As/Save All/Revert (save pipeline), dirty prompts, hot exit, tabs (states, overflow, pin, context menu), reopen closed tab, encoding/EOL pickers, CLI args, single-instance socket | 02, 04, 08, 09 (save), 31 (single instance) | Todo | |
| M6 | Splits & groups: split 4 directions, duplicate view, tab drag reorder + docking targets, move between groups, maximize group, single editor | 03, 04 | Todo | |
| M7 | Workspace & Explorer: open/close workspace, lazy tree + background index, filter, hidden/ignored, file ops (new/rename/duplicate/delete→trash/copy/drag), reveal, recent workspaces + pins, workspace trust | 07, 08, 31 | Todo | |
| M8 | Reliability: recovery snapshots + startup prompt/review/discard, autosave modes, file watcher + external change banner + self-write suppression, deleted/read-only transitions, session save/restore per workspace, crash test harness | 09, 10, 23, 33 (reliability) | Todo | Recovery must be complete before daily use. |
| M9 | Find/replace widget, Go to Line, Command Palette foundation (+ pickers), configurable keybindings file, minimap | 11, 13, 15 (go to line), 16 | Todo | |
| M10 | Settings tab (searchable, User/Workspace scopes, keyboard shortcuts editor), first-run page, Zen Mode, full screen, status bar pickers complete | 14, 17, 19, 20 | Todo | **Milestone "daily editor" (spec 00 / requirement §89).** |
| M11 | Packaging: cargo-packager configs (msi/nsis, app/dmg universal, AppImage/deb), release workflow, checksums, optional signing | 31 | Todo | |

## Phase 2 — Structured documents

| # | Milestone | Specs | Status | Notes |
|---|---|---|---|---|
| M12 | Syntax highlighting (syntect packdump, incremental, theme mapping), language registry + picker | 26 | Todo | |
| M13 | Markdown: preview renderer, Edit/Split/Preview, scroll sync, links, local images, task toggles, editing aids | 24 | Todo | |
| M14 | JSON: span parser, Tree view, Text/Split/Tree, selection sync, format/minify/sort, errors | 25 | Todo | |
| M15 | Outline panel, breadcrumbs, Go to Symbol, folding providers, bookmarks | 12, 26 | Todo | |
| M16 | Quick Open (index, fuzzy, `file:line`, prefixes), workspace search panel + replace, navigation history | 15 | Todo | |
| M17 | Compare: sources, diff model, diff view, hunk copy, Compare Current Changes; multi-cursor + Selection menu | 27, 11 | Todo | |

## Phase 3 — Git

| # | Milestone | Specs | Status | Notes |
|---|---|---|---|---|
| M18 | Git read: detection, status, decorations (Explorer/tabs/gutter/minimap), status bar branch, compare with HEAD/revision, file history | 28 | Todo | |
| M19 | Git write: Source Control panel (stage/unstage/discard/commit), branches, fetch/pull/push/sync via system git, clone/init, Git Output, Sync Notes | 28 | Todo | |

## Phase 4 — MCP

| # | Milestone | Specs | Status | Notes |
|---|---|---|---|---|
| M20 | MCP server (rmcp streamable HTTP, tokens, Host/Origin checks), permission gate + Ask prompts, connections/permissions/activity UI, `mcp-bridge` | 29, 32 | Todo | |
| M21 | MCP tools: workspace, document, editor, settings, filesystem, git read; revision conflicts; edit highlighting | 29 | Todo | |

## Phase 5 — Lua plugins

| # | Milestone | Specs | Status | Notes |
|---|---|---|---|---|
| M22 | Plugin host: mlua sandbox, manifest, approval, permissions, lifecycle, commands/keybindings/menus, events, notifications, errors/disable | 30, 32 | Todo | |
| M23 | Plugin UI integration: status items, declarative panels, formatters, outline providers, languages, plugin settings, Plugin Manager, Developer Tools, example plugins | 30 | Todo | |

## Phase 6 — Advanced (candidates, re-planned when reached)

| # | Milestone | Specs | Status | Notes |
|---|---|---|---|---|
| M24 | Wiki links, backlinks, tags, link index | 34 | Todo | |
| M25 | Indexed search, templates | 34 | Todo | |
| M26 | Custom themes, Markdown export, JSON tree editing, updater | 21, 24, 25, 31 | Todo | |
