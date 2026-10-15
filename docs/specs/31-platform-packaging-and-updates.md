# 31 — Platform Abstraction, Packaging & Updates

**Status:** Partial — app paths, system fonts (M0); async file dialogs, reveal in file manager, single instance (M5); folder picker, move to trash (M7); packaging: cargo-packager config (identifier, icons, document types), `dist` profile with split debug info, `scripts/package.sh` (universal macOS .app/.dmg, .msi + NSIS, .deb + AppImage, checksums, debug-symbol archives, optional signing/notarization/GPG), tag-triggered release workflow (M11). macOS packaging verified locally; Windows and Linux formats are exercised by the release workflow. Finder "Open With" (Apple Events) not handled yet; updates (M26)

## Purpose

Define the cross-platform abstraction layer (`notoir-platform`), per-OS expectations, application paths, packaging and
the (optional, later) update mechanism.

## Platform Abstraction

```rust
pub struct AppPaths { pub config: PathBuf, pub data: PathBuf, pub cache: PathBuf, pub logs: PathBuf, pub recovery: PathBuf, pub plugins: PathBuf, pub scratchpads: PathBuf, pub themes: PathBuf }

pub trait Platform {
    fn paths(&self) -> &AppPaths;
    fn pick_files(&self, opts: FileDialogOpts) -> Option<Vec<PathBuf>>;   // rfd, called from UI via async rfd to avoid blocking
    fn pick_folder(&self, opts: FolderDialogOpts) -> Option<PathBuf>;
    fn save_file(&self, opts: SaveDialogOpts) -> Option<PathBuf>;
    fn reveal(&self, path: &Path) -> Result<(), PlatformError>;          // select in file manager
    fn open_url(&self, url: &str) -> Result<(), PlatformError>;
    fn open_path(&self, path: &Path) -> Result<(), PlatformError>;       // default app
    fn move_to_trash(&self, paths: &[PathBuf]) -> Result<(), PlatformError>;
    fn system_fonts(&self) -> &fontdb::Database;
    fn os_locale(&self) -> Option<String>;
    fn prefers_dark(&self) -> Option<bool>;
    fn reduce_motion(&self) -> bool;
    fn default_line_ending(&self) -> LineEnding;                         // CRLF on Windows, LF elsewhere
    fn reveal_label_key(&self) -> &'static str;                          // "Reveal in File Explorer" / "Reveal in Finder" / "Open Containing Folder"
}
```

A `FakePlatform` implementation (temp dirs, scripted dialog answers) is used in tests.

Dialogs use `rfd::AsyncFileDialog` driven on the Tokio runtime so the UI thread is never blocked (macOS requires
dialogs on the main thread: rfd handles this by dispatching; verified in M0).

## Paths

Via `directories::ProjectDirs` with a bundle identifier fixed in M0 (placeholder `dev.notoir.Notoir`; **must never
change after the first release**, otherwise user data is orphaned — a migration routine would be required).

| Purpose | Windows | macOS | Linux |
|---|---|---|---|
| config (settings, keybindings, themes) | `%APPDATA%\Notoir\config` | `~/Library/Application Support/<id>/config` | `$XDG_CONFIG_HOME/notoir` |
| data (state.db, recovery, scratchpads, plugins) | `%APPDATA%\Notoir\data` | `~/Library/Application Support/<id>` | `$XDG_DATA_HOME/notoir` |
| cache (syntax packdump, image cache, file index) | `%LOCALAPPDATA%\Notoir\cache` | `~/Library/Caches/<id>` | `$XDG_CACHE_HOME/notoir` |
| logs | `%LOCALAPPDATA%\Notoir\logs` | `~/Library/Logs/Notoir` | `$XDG_STATE_HOME/notoir/logs` |

`NOTOIR_HOME` env var overrides all roots (portable mode, tests). No hard-coded Unix paths anywhere.

## Reveal / Open

| OS | Reveal | Open default |
|---|---|---|
| Windows | `explorer.exe /select,"<path>"` | `open` crate (ShellExecute) |
| macOS | `open -R <path>` | `open` |
| Linux | D-Bus `org.freedesktop.FileManager1.ShowItems`; fallback `xdg-open <parent dir>` | `xdg-open` |

## Platform Expectations

### Windows (10/11)

Standard file pickers; Explorer integration (reveal, "Open with Notoir" registration by installer); path semantics
(drive letters, UNC, `\\?\` long paths, case-insensitive identity, reserved names); CRLF awareness (default EOL for new
files, `files.eol = auto`); per-monitor DPI scaling (eframe/winit); native window behaviours (snap, title bar via winit
decorations); `Ctrl` shortcuts; Alt key reveals menu mnemonics.

### macOS (Apple Silicon + Intel universal build)

`Cmd` conventions; app menu (spec 18); Finder reveal; Retina; `~/Library` paths; full screen via native green button;
`application:openFiles` for Finder "Open With"; app bundle with `Info.plist` document types (txt, md, json, …); Option
key as Alt-equivalent; respect "Prefer tabs" off (Notoir manages its own tabs); quit via Cmd+Q triggers hot exit/prompts.

### Linux (mainstream: Ubuntu, Fedora, Debian, Arch; GNOME and KDE)

X11 and Wayland (winit features both enabled); fontconfig-driven font discovery via fontdb; XDG directories;
`xdg-open` / FileManager1; inotify limits handling (spec 10); `.desktop` file with MIME types; HiDPI fractional scaling
as reported by winit.

## Single Instance

A per-user local socket (`interprocess` crate: named pipe on Windows, Unix domain socket elsewhere) at
`<data>/notoir.sock`. Launching `notoir file.md` while an instance runs forwards the request and exits; `--new-window`
bypasses forwarding. Stale sockets (no listener) are replaced.

## Packaging

`cargo-packager` (as in string_art_it_desktop) configured in `apps/notoir/Cargo.toml`.

| OS | Formats | Signing |
|---|---|---|
| Windows | `.msi` (WiX) and NSIS `.exe` | Authenticode when certificate available (CI secret); unsigned builds marked "preview" |
| macOS | `.app`, `.dmg` (universal2) | Developer ID signing + notarization + stapling in CI when secrets present; hardened runtime |
| Linux | AppImage, `.deb`; Flatpak manifest later | AppImage signed with GPG detached signature |

- Release profile: thin LTO, `codegen-units = 1`, strip symbols; separate debug symbols archived for crash analysis.
- CI release workflow (GitHub Actions) builds all formats on tag push; artifacts + SHA-256 checksums attached to the
  release. Signing steps are skipped gracefully without secrets.
- File associations and icons generated from `assets/app-icon/notoir.svg`.

## Updates (optional, Phase 6)

- Off unless enabled; `update.mode = off | notify | download` (default `notify` for release builds — checks a signed
  JSON manifest over HTTPS at most daily; no telemetry beyond the request itself).
- Manifest and artifacts signed with an Ed25519 key (minisign format); the public key is embedded in the binary;
  signature and version (must be greater, semver) verified before anything is offered.
- Install flow is non-destructive: download to cache, verify, then hand off to the platform installer (msi / dmg
  replace / AppImage swap) **after** Notoir exits normally with hot exit (no data loss). Never auto-install silently.
- Linux distro packages (deb/Flatpak) defer to the package manager (updater disabled).

## Test Cases

```gherkin
Feature: Paths

  Scenario: NOTOIR_HOME overrides all directories
    Given NOTOIR_HOME is set to a temp directory
    When Notoir starts
    Then config, data, cache, logs, and recovery directories are created under it

Feature: Reveal

  Scenario: Platform label
    Given the app runs on macOS
    Then the tab context menu shows "Reveal in Finder"
    Given the app runs on Windows
    Then it shows "Reveal in File Explorer"

Feature: Single instance

  Scenario: Second launch opens file in running instance
    Given Notoir is running
    When the user runs "notoir notes.md" in a terminal
    Then "notes.md" opens in the running window
    And no second window appears

Feature: Updates

  Scenario: Invalid signature is rejected
    Given an update manifest with an invalid signature
    When Notoir checks for updates
    Then no update is offered and a warning is logged
```
