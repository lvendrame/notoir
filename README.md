# Notoir

A fast, native, cross-platform text editor and workspace application written in Rust (egui/eframe).

> Plain files + Git + MCP + native editor.

Notoir opens a `.txt` file and lets you type immediately, like a notepad. When you need more, it also supports
workspaces, split editors, Markdown preview, JSON tree views, file comparison, Git, an MCP server, and Lua plugins.
None of these take up permanent screen space until you use them.

Runs on Windows 10/11, macOS (Apple Silicon and Intel), and Linux (X11/Wayland). Available in 14 languages.

## Status

Under development: a daily-usable editor (files, workspaces, splits, recovery, find/replace, settings) with
Markdown, JSON, Git, MCP and plugins still to come. See:

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

## Packaging

Release packages are built per OS with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) 0.11
(`cargo install cargo-packager --version 0.11.8 --locked`):

```sh
scripts/package.sh   # macOS: universal .app + .dmg · Windows (Git Bash): .msi + NSIS · Linux: .deb + AppImage
```

Output, `SHA256SUMS-<os>.txt` and debug symbols land in `target/packages`. Signing runs only when the secrets named at
the top of the script are set; unsigned builds are previews. Pushing a `v*` tag runs `.github/workflows/release.yml`,
which packages on all three OSes and publishes a GitHub release with combined checksums. The crash suite runs nightly
(`cargo test -p notoir --test crash -- --ignored`).
