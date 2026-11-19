# v0.1.14 release preparation

Prepared on 2026-11-19. Publication requires a fresh successful manual CI run on
the final candidate, including Windows, macOS and Linux crash suites. Record the
candidate SHA and successful CI URL in the published release notes.

## Changes

- Align submenu and dynamic-entry labels with command labels in in-window menus
  and context menus; preserve native macOS menu rendering.
- Reject stale background theme scans after save/import/remove or a newer scan,
  preventing imported styles from being replaced by an older directory snapshot.
- Upgrade zip to 9.0.5 and libz-sys to 1.1.33 for their RustSec advisories.
- Replace direct chrono usage with jiff 1 and upgrade trash to 5.4.0.
- Temporarily pin the chrono-optional MCP SDK patch to
  `9043cdfa8694d83bf506c2f3eb0313ec173edd7e` from
  <https://github.com/andreinknv/rust-sdk>. The patch is linked from upstream
  <https://github.com/modelcontextprotocol/rust-sdk/issues/1469> and was reviewed
  against published rmcp 3.11.0. Default features stay disabled. Replace this pin
  with an official release when chrono becomes optional; do not enable the chrono
  or auth features. No advisory suppression was added.

## Local validation before commit

- `cargo deny --all-features check`: advisories, bans, licenses and sources pass.
- Dependency tree with all root features and targets contains no chrono.
- `cargo fmt --check` and `git diff --check`: pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo test --workspace --no-fail-fast`: 859 passed, 6 ignored, no failures.
- Theme Manager regressions: 16 passed, including import/export and stale scans.
- `cargo test -p notoir --test crash -- --ignored --test-threads=1`: 3 passed.
- Store-edition policy test and Node Store CI-gate tests: pass.
- `cargo build -p notoir`: pass.
- `scripts/live-windows.py`: 10 native macOS lifecycle checks passed.
- `scripts/live-native-menus.py`: 7 native AppKit checks passed. Run separately
  from other native GUI suites; concurrent crash-test apps took focus during the
  first attempt and closed a tracked menu early. The isolated rerun passed.

The target directory was cleaned before rebuilding. Native Windows/Linux checks,
Windows packaging regressions and cross-platform validation are performed by the
required manual CI run after the candidate commit is pushed.
