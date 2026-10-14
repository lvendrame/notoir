# Notoir

Native Rust + egui/eframe cross-platform text editor and workspace application (Windows, macOS, Linux).
Functional specs: [`docs/specs/`](docs/specs/) — normative. Entry point: [`00-overview-and-scope.md`](docs/specs/00-overview-and-scope.md).
Architecture: [`01-architecture.md`](docs/specs/01-architecture.md). Design system: [`21-themes-and-design-system.md`](docs/specs/21-themes-and-design-system.md).

## Start here

Read [`docs/plan/orchestrator.md`](docs/plan/orchestrator.md). Pick the first milestone not `Done`, read its specs,
turn their Gherkin scenarios into Rust tests, implement, verify, then update the status row with notes.

Spec traceability ([`docs/plan/spec-coverage.md`](docs/plan/spec-coverage.md), generated): tag each test that covers a
scenario with `// spec NN: "Exact scenario title"`, keep each spec's `**Status:**` line current, and run
`cargo run -p spec-check -- --write`. `cargo test` fails when tags, statuses and the report disagree. Only tag a test
when it asserts every `Then` of the scenario (at least at model level).

## Layout

- `crates/notoir-core` — domain: documents registry, editor views, groups/layout tree, commands, events, settings model,
  permissions model, workspace model. No egui, no filesystem/network IO.
- `crates/notoir-editor` — text engine: rope buffer ops, transactions/undo, selections, search, folding, outline,
  syntax, diff, Markdown/JSON structure. No egui, no IO.
- `crates/notoir-storage` — settings files (TOML), SQLite state store + migrations, recovery store, recent lists.
- `crates/notoir-workspace` — workspace scan, file watching, workspace search, fuzzy file index.
- `crates/notoir-platform` — app paths, dialogs, reveal/open, system fonts, single-instance, trash.
- `crates/notoir-git` — git2 (local) + system git (network) adapter.
- `crates/notoir-mcp` — rmcp server, tool schemas, transport, bridge.
- `crates/notoir-plugins` — mlua host, manifest, plugin API.
- `apps/notoir` — eframe binary: shell, theme, icons, i18n, widgets, editor widget, panels.
- `tools/spec-check` — spec status / scenario-coverage checker and report generator.

`notoir-core` and `notoir-editor` must never depend on egui, tokio, or IO crates (enforced by
`crates/notoir-core/tests/layering.rs`).

Crates are created by the milestone that first needs them (storage M2, workspace M7, git M18, mcp M20, plugins M22) —
no empty placeholder crates.

## Assets

- Icons: `assets/icons/lucide/*.svg` pinned to lucide-static **1.48.0**, plus `assets/icons/custom/`. Add icons by
  copying from lucide-static 1.48.0 and adding a row to `icons!` in `apps/notoir/src/icons.rs`.
- Fonts: Manrope (UI) + JetBrains Mono (editor, numeric readouts) in `assets/fonts/`; CJK via system fonts.
- App icons: `assets/app-icon/` generated from `notoir.svg` (`cargo run -p notoir --example gen_icons`, then the
  `iconutil` / `magick` commands in that example's doc comment).
- Themes: `assets/themes/*.toml` (spec 21 format); every `Tok` in `apps/notoir/src/theme.rs` must be defined.
- Locales: `assets/locales/<lng>/<ns>.json` (14 languages, i18next-style JSON). Keep all in sync; key-parity test enforces it.

## Rules

- Every user-visible string goes through i18n (`t!("ns:key")`). Never translate user content or the brand name.
- UI, menus, toolbar, shortcuts, palette, plugins and MCP all go through the command registry — no duplicated logic.
- Never block the UI thread on IO, Git, search or plugin work.
- Never write a user file except through the save pipeline (spec 09).

## Manual checks

- `NOTOIR_HOME=<dir>` relocates all app directories; `NOTOIR_LANG=<code>` / `NOTOIR_THEME=<id>` override language/theme
  until the settings UI exists; `NOTOIR_LOG=debug` raises log verbosity.
- `cargo test -p notoir --test render` writes PNG snapshots to `target/ui-snapshots`.

## Before considering any change done

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
