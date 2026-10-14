# 33 — Testing & CI

**Status:** Partial — CI workflow, kittest and render snapshots (M0); crash suite (M8), performance budgets (M4)

## Purpose

Define the testing strategy (unit, integration, UI, visual snapshots, reliability/crash tests, performance budgets) and
cross-platform CI. Every spec's Gherkin scenarios are the acceptance list for its milestone; each scenario maps to at
least one Rust test (named after the scenario, e.g. `fn dirty_file_is_never_silently_reloaded()`).

## Test Layers

| Layer | Location | Tools | Covers |
|---|---|---|---|
| Unit | `crates/*/src/**` `#[cfg(test)]` | std test, `proptest` for buffer/diff/display-map invariants, `insta` for structured snapshots | settings resolution, workspace overrides, dirty state, transactions/undo, selections transform, display map, recovery format, command registry/when-expressions, keybinding resolution, path normalization, recent list, diff model, Git status mapping, MCP permission checks, plugin permission checks, i18n detection/plurals |
| Integration | `crates/*/tests/` and workspace `tests/` | temp dirs (`tempfile`), `FakePlatform`, real git repos created in temp dirs, rmcp client | open/edit/save, atomic save, recovery round trip, workspace open, external modifications (real watcher), Git workflows against a local bare remote, MCP document/filesystem operations over HTTP, plugin lifecycle with example plugins, session persistence + migrations |
| UI | `apps/notoir/tests/` | `egui_kittest` harness driving `NotoirApp` (as string_art_it_desktop) | tabs, splits, duplicate view, context menus, palette, find widget, settings tab, empty state, first run, language switching, keyboard shortcuts per platform modifier |
| Visual snapshots | `apps/notoir/tests/render.rs` | kittest + wgpu render to PNG in `target/ui-snapshots` (`NOTOIR_SNAPSHOT_DIR`) | key screens × 3 themes × (en, ja, de) for review; not pixel-asserted in CI (GPU variance), reviewed in PRs |
| Reliability | `tests/crash/` | spawn the real binary with `NOTOIR_HOME` temp, script edits via MCP or a test-only command channel, `kill -9`, relaunch | crash with unsaved files, crash with Scratchpads, crash during save, conflicting recovery snapshot, corrupt state.db, disk full (via small tmpfs on Linux CI), read-only file, deleted file, workspace disappearing |
| Performance | `benches/` (criterion) + perf smoke tests | criterion; frame-time measurement in kittest | typing latency, scrolling 1M-line file, minimap rebuild, syntax incremental update, workspace scan 100k files, search throughput, startup time |

Layer boundary test (spec 01) and i18n key parity + string-literal lint (spec 22) run as normal tests.

## Required Scenario Coverage (from product requirements)

Crash with unsaved files · crash with Scratchpads · external file changes · disk write failure · file becomes
read-only · file deleted externally · workspace disappears · Git operation changes open file · MCP changes active
document · plugin fails during callback · malformed plugin · invalid JSON · invalid/odd Markdown (fuzzed with
`proptest` strings — preview must not panic) · encoding problems (lossy decode, UTF-16, BOM) · conflicting recovery
snapshot · suspend/resume (simulated frame-time gap).

## Performance Budgets (enforced as smoke tests with generous CI margins)

| Metric | Budget (release build, mid-range laptop) |
|---|---|
| Cold start to first frame, no session | < 300 ms |
| Restore session with 10 tabs | < 600 ms to interactive |
| Keystroke to paint, 10k-line file | < 8 ms frame |
| Scroll frame, 1M-line plain file | < 8 ms |
| Open 50 MiB file | interactive < 1.5 s (loading off-thread) |
| Workspace scan 100k files | non-blocking; complete < 3 s on SSD |
| Idle CPU | ~0 % (egui repaints only on events/animations) |

## CI (GitHub Actions)

- Matrix: `ubuntu-latest`, `windows-latest`, `macos-latest` (arm64) + `macos-13` (x86_64) for release builds.
- Jobs: `fmt` (`cargo fmt --check`), `clippy` (`-D warnings`, all targets), `test` (workspace; kittest UI tests run
  headless — wgpu snapshot tests skip gracefully without an adapter, as in the reference project), `deny`
  (`cargo deny check` licenses/advisories/bans), `audit`, `msrv` (rust-version in workspace manifest), `docs-links`
  (spec cross-reference checker).
- Linux CI installs: `libxkbcommon-dev`, `libgtk-3-dev` (rfd), `libwayland-dev`, fontconfig; Git installed for Git tests.
- Caching via `Swatinem/rust-cache`.
- Release workflow on tags (spec 31).
- Nightly job: reliability/crash suite + performance smoke tests on all three OSes.

## Test Conventions

- Tests never touch the real user directories: `NOTOIR_HOME` / `FakePlatform` always.
- No network in tests (Git remote is a local bare repo; MCP on loopback with ephemeral port).
- Deterministic time via an injectable `Clock` for debounce/autosave/recovery scheduling.
- Each milestone closes only when its spec scenarios have tests and all three OS jobs are green.

## Test Cases

```gherkin
Feature: CI gates

  Scenario: Pull request checks
    When a pull request is opened
    Then fmt, clippy, test, deny, and audit run on Windows, macOS, and Linux
    And the pull request cannot merge while any of them fails
```
