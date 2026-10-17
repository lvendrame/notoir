# Troubleshooting and data locations

## Where Notoir keeps its data

| Folder | Holds | Windows | macOS | Linux |
|---|---|---|---|---|
| config | `settings.toml`, `keybindings.toml`, `themes/`, `templates/` | `%APPDATA%\Notoir\Notoir\config` | `~/Library/Application Support/dev.Notoir.Notoir/config` | `~/.config/notoir` |
| data | session and state database, recovery snapshots, scratchpads, `plugins/` | `%APPDATA%\Notoir\Notoir\data` | `~/Library/Application Support/dev.Notoir.Notoir` | `~/.local/share/notoir` |
| cache | search index, update downloads | `%LOCALAPPDATA%\Notoir\Notoir\data\cache` | `~/Library/Caches/dev.Notoir.Notoir` | `~/.cache/notoir` |
| logs | log files | `%LOCALAPPDATA%\Notoir\Notoir\data\logs` | `~/Library/Logs/Notoir` | `~/.local/state/notoir/logs` |

On Linux the `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` variables are respected.
Workspace-specific files live in the workspace, in `.notoir/` (`settings.toml`, `plugins.toml`, `templates/`).

Deleting the cache folder is always safe. Deleting the data folder resets sessions, scratchpads, recovery snapshots,
installed plugins and MCP clients.

## Environment variables

| Variable | Effect |
|---|---|
| `NOTOIR_HOME=<dir>` | keep all four folders under `<dir>` instead (handy for a clean test profile) |
| `NOTOIR_LANG=<code>` | use this display language, e.g. `NOTOIR_LANG=ja` |
| `NOTOIR_THEME=<id>` | use this theme, e.g. `NOTOIR_THEME=notoir-paper` |
| `NOTOIR_LOG=debug` | write more detail to the log |

```sh
NOTOIR_HOME=$(mktemp -d) notoir     # start with a fresh, empty profile
```

## Logs

Notoir writes a log file to the logs folder; the level is set by `logging.level` (or `NOTOIR_LOG`). When reporting a
problem, attach the latest log. User-facing errors also appear as notifications; **Show Details** shows the technical
reason.

## Common problems

**macOS says "Notoir" Not Opened, or Windows says it protected your PC.** Release builds are not code-signed yet. See
[Installing Notoir](install.md).

**My settings change does nothing.** Check the warning notification: a syntax error in `settings.toml` keeps the last
good values. A workspace setting may override your user setting (the Settings tab shows *Overridden in Workspace*), and
restricted settings in a workspace need [trust](workspaces.md#workspace-settings-and-trust).

**A shortcut does not work.** Search it in **Settings › Keyboard**: another command may use the same keys (conflicts
are marked), or the binding's `when` condition does not apply where you are.

**Search results look outdated or incomplete.** Run **Rebuild Search Index** from the command palette. Check
`files.exclude`, `search.exclude` and `.gitignore`, and `search.maxFileSize` for big files.

**An AI client cannot connect.** Make sure the server is running (**MCP ●** in the status bar) and that the port in the
client's configuration matches (it changes if 47823 was taken). A revoked client needs a new token: use **Copy Setup
Command** again. **MCP Activity** shows rejected requests and why.

**A plugin stopped working.** Look in **Plugins › Plugin Output** and the Plugin Manager: it may be disabled after
errors, or need a review after an update.

**Notoir crashed.** Start it again: your unsaved work is offered back (see
[Crash recovery](files.md#crash-recovery)).

## Reporting a bug

Open an issue at <https://github.com/lvendrame/notoir/issues> with your system, the Notoir version (**Help › About
Notoir**), the steps that cause the problem and, if possible, the log file.
