# Notoir

A fast, native, cross-platform text editor and workspace application written in Rust (egui/eframe).

> Plain files + Git + MCP + native editor.

Notoir opens a `.txt` file and lets you type immediately, like a notepad. When you need more, it also supports
workspaces, split editors, Markdown preview, JSON tree views, file comparison, Git, an MCP server, and Lua plugins.
None of these take up permanent screen space until you use them.

Runs on Windows 10/11, macOS (Apple Silicon and Intel), and Linux (X11/Wayland). Available in 14 languages.

## Install

Download the installer for your system from the [latest release](https://github.com/lvendrame/notoir/releases/latest):

| System | File |
|---|---|
| macOS (Apple Silicon and Intel) | `Notoir_<version>_universal.dmg` |
| Windows | `notoir_<version>_x64_en-US.msi` or `notoir_<version>_x64-setup.exe` |
| Linux | `notoir_<version>_amd64.deb` or `notoir_<version>_x86_64.AppImage` |

Release builds are not code-signed yet, so the first launch needs one extra step:

- **macOS** says "Notoir" Not Opened. Drag Notoir to Applications, then run:
  ```sh
  xattr -dr com.apple.quarantine /Applications/Notoir.app
  ```
  (or click **Open Anyway** in System Settings → Privacy & Security).
- **Windows** SmartScreen says "Windows protected your PC": click **More info** → **Run anyway**.
- **Linux** AppImage: `chmod +x notoir_*.AppImage` before running it.

`SHA256SUMS.txt` in each release lists the checksum of every file.

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
the top of the script are set; unsigned builds are previews. The crash suite runs nightly
(`cargo test -p notoir --test crash -- --ignored`).

## Releasing

1. Set `version` in the root `Cargo.toml` (e.g. `0.2.0`), commit, push.
2. Either push a tag (`git tag v0.2.0 && git push origin v0.2.0`), or run Actions → Release → Run workflow with
   **publish** ticked: it tags the chosen commit `v<Cargo version>` (refusing an existing tag).

`.github/workflows/release.yml` then checks that the tag matches the Cargo version, packages on all three OSes and
publishes a GitHub release with every installer, `SHA256SUMS.txt`, debug symbols and, when signing is set up, a signed
`update.json`. Tags containing `-` become prereleases; the part after `-` must be a number (`v0.2.0-1`), because MSI
versions are numeric. A manual run without **publish** only builds the packages as workflow artifacts.

Optional repository secrets (each step is skipped without them):

| Secrets | Effect |
|---|---|
| `APPLE_CERTIFICATE` (base64 .p12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`; `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_PASSWORD` | macOS signing; notarization |
| `WINDOWS_CERTIFICATE` (base64 .pfx), `WINDOWS_CERTIFICATE_PASSWORD` | Authenticode for .msi and setup .exe |
| `GPG_PRIVATE_KEY`, `GPG_KEY_ID` | AppImage `.asc` signature |
| `MINISIGN_SECRET_KEY` (password-less, `minisign -G -W`) + variable `NOTOIR_UPDATE_PUBKEY` | Signed `update.json`; enables the in-app updater |

`NOTOIR_UPDATE_URL` (variable) defaults to the latest release's `update.json`.
