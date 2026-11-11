# Theme manager implementation handoff

Implementation complete for the approved M26 theme-management follow-up.

- Tools → Manage Themes opens a native editor tab with grouped colors, typography, display controls, language/extension rules and a real editor preview.
- Built-ins store editable overrides separately and can be reset. Custom themes support creation, rename, duplication, activation, deletion and native TOML import/export.
- Drafts preview locally; background atomic writes detect disk conflicts. Saved changes refresh sibling windows, and cancelling a dirty-theme close cancels shutdown.
- Explicit User/Workspace settings retain precedence. Installed fonts include proportional editor fonts; text geometry is shared by wrapping, painting, pointer/caret/IME positions and selections. Wrapping remeasures edited lines incrementally.
- Main implementation: `apps/notoir/src/theme_manager.rs`, `apps/notoir/src/shell/themes_ui.rs`, and `apps/notoir/src/editor_widget/geometry.rs`, with integration in settings, commands, theme loading, fonts and window lifecycle.

## Verification

Passed `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` on macOS. Focused final checks also passed: 15 theme-manager tests, 8 window lifecycle tests, editor/selection/split/syntax tests and measured-wrap cache invalidation. Generated documentation and spec coverage are current (280/307 scenarios; eight added theme scenarios covered).

GPU screenshots were rendered, visually reviewed and copied to `docs/design/theme-manager*.png` for normal, file-rule and narrow layouts. Full workspace log: `/tmp/notoir-theme-manager-workspace-tests.log`.

Native Windows/Linux acceptance and the pre-existing ignored crash/live-network suites were not run; platform acceptance remains under W7. No release, commit or remote publication was performed.
