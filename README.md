# Notoir

A fast, native, cross-platform text editor and workspace application written in Rust (egui/eframe).

> Plain files + Git + MCP + native editor.

Notoir opens a `.txt` file and lets you type immediately, like a notepad. When you need more, it also supports
workspaces, split editors, Markdown preview, JSON tree views, file comparison, Git, an MCP server, and Lua plugins.
None of these take up permanent screen space until you use them.

Runs on Windows 10/11, macOS (Apple Silicon and Intel), and Linux (X11/Wayland). Available in 14 languages.

## Status

Specification phase. See:

- [`docs/specs/`](docs/specs/): functional and technical specifications (start with
  [`00-overview-and-scope.md`](docs/specs/00-overview-and-scope.md))
- [`docs/plan/orchestrator.md`](docs/plan/orchestrator.md): milestone plan and status

## Development

```sh
cargo run -p notoir
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
