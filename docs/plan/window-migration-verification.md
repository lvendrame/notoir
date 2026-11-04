# Single-process window migration verification

The normative contract is [spec 40](../specs/40-application-and-window-lifecycle.md). Architectural decisions AD-10 through AD-15 describe the process/window split, native menus, persistent identities, shared MCP transport and wgpu renderer.

## Local checks

- `cargo fmt --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: passed, including the new host, MCP targeting, shared-settings serialization, persistence-failure and launch-gate coverage.
- `cargo run -p spec-check -- --write`: regenerated specification coverage without contradictions.
- `cargo run -p notoir --example gen_docs`: regenerated menu, command and MCP references.

The filesystem watcher and local socket tests require normal macOS event/socket access; the full suite was run outside the restricted execution sandbox. A sandboxed targeted watcher run failed to receive an event; the unrestricted full-suite watcher scenario and the separate 28-test window/workspace rerun passed.

## Native macOS acceptance

Run `cargo build -p notoir`, then `python3 scripts/live-windows.py --performance --artifacts target/live-windows-final` from a graphical macOS session. The script creates a disposable profile and records snapshots, logs and results. It does not modify the normal Notoir profile.

All ten checks passed:

1. The initial native editor reports the application PID.
2. CLI New Window uses the same PID.
3. The second window has its own tabs and content.
4. UI New Window creates an empty context.
5. Window activation switches editing context.
6. Closing the first-created editor preserves its siblings.
7. Application Quit exits cleanly.
8. Restart restores the remaining windows and their identities.
9. macOS stays running after all editors close.
10. A new-window launch recreates an editor in that existing process.

The native run exposed two integration issues now addressed: OpenGL context/texture failures were resolved by selecting wgpu explicitly, and stale native focus observations no longer override a requested window activation while the focus event is in flight. A hidden root has a logic-only wake path; native windows are materialized before immediate rendering.

## Idle samples and limits

The acceptance runner collected two-second samples on the local machine after settling:

| Editor windows | CPU seconds | Percent of one CPU core |
|---|---|---|
| 1 | 0.23 | 11.5% |
| 3 | 0.27 | 13.4% |
| 10 | 0.37 | 18.4% |

These are short observations with normal cursor/housekeeping activity, not cross-platform performance budgets. Immediate viewports couple repaint work; the architecture records that tradeoff. IPC, menu actions and background completions wake the host directly, with a one-second housekeeping fallback.

Native Windows/Linux window-manager behavior, physical macOS menu/Dock interactions and extended crash/performance stress runs still require their graphical environments. The checked-in native script drives the command registry and verifies real native editor windows; it does not automate OS menu clicks. Spec 40 remains Partial rather than claiming all its Gherkin scenarios have automated coverage.

## Native menu tracking regression

The first implementation compared `Debug` strings of freshly allocated `ContextSnapshot` hash maps. Different iteration order made an unchanged menu appear different, so every editor update removed/recreated the entire native bar. This dismissed AppKit's tracking session. The zero-window context was empty, which explains why Quit worked after closing the editor.

Native menus now resolve semantic item properties and retain their native bar/submenu/action objects. Enablement, checkmarks, localized labels and zero-window accelerators update in place. Actual dynamic child changes are confined to their owning submenu. Context map ordering and unrelated cursor state never determine native structure.

Run `python3 scripts/live-native-menus.py` after building the debug app. It loads the checked-in Objective-C probe only into an isolated Notoir test process, retains native object references across editor frames, and holds real AppKit popup tracking sessions open for 1.5 seconds. It needs no Accessibility permission or global input events. The native probe passed all seven checks: persistent native identities, File tracking with an editor, Notoir tracking with an editor, native New Window dispatch, closing all editors, Notoir tracking without editors, and native Quit dispatch followed by a clean process exit. Artifacts are in `target/live-native-menus`. Unit regressions additionally cover reordered/recreated contexts, enablement, localization, checkmarks and dynamic submenu boundaries.

Validation after the menu fix: formatting and workspace Clippy pass; the serial full workspace run (`cargo test --workspace -- --test-threads=1`) passes 795 tests. The initial concurrent run hit the existing asynchronous Git preview timing race; that test passed unchanged on its targeted rerun and in the full serial run. Native AppKit probe results are recorded in `target/live-native-menus/results.json`.
