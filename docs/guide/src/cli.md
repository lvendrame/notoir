# Command line

```text
notoir [paths…] [--new-window] [--goto file:line[:col]] [--wait]
notoir mcp-bridge --token <token> [--port <port>]
```

| Option | Meaning |
|---|---|
| `paths…` | files to open as documents and folders to open as workspaces, relative to the current directory |
| `-n`, `--new-window` | open a new window instead of using the running one |
| `-g`, `--goto <file:line[:col]>` | open a file at a line and optional column |
| `-w`, `--wait` | wait until the files opened by this command are closed |
| `-h`, `--help` | show the help (in your system language) |
| `-V`, `--version` | print `Notoir <version>` |
| `--` | treat everything after it as paths |

Without paths, Notoir restores the last session.

## Examples

```sh
notoir .                         # open the current folder as a workspace
notoir notes.md todo.txt         # open two files
notoir --goto src/main.rs:12:4   # open at line 12, column 4
notoir --new-window ~/projects/blog
```

If Notoir is already running, the command hands the files to the most recently used window and returns at once. A
path containing colons (such as a Windows drive) is kept whole by `--goto`.

## Using Notoir as your editor

With `--wait`, the command returns only when you close the tabs of the files it opened. That makes Notoir usable
wherever a program starts an editor and waits:

```sh
git config --global core.editor "notoir --wait"
export EDITOR="notoir --wait"     # in your shell profile
```

## Installing the command

| Installed with | The `notoir` command |
|---|---|
| Linux `.deb` | installed as `/usr/bin/notoir` |
| macOS app | **Help › Install 'notoir' Command in PATH** creates `/usr/local/bin/notoir` (asks for your password when needed) |
| AppImage | **Help › Install 'notoir' Command in PATH** links `~/.local/bin/notoir` to the AppImage |
| Windows | **Help › Install 'notoir' Command in PATH** adds the install folder to your user `PATH` (open a new terminal afterwards) |

**Help › Uninstall 'notoir' Command from PATH** removes it again.

## `notoir mcp-bridge`

A stdio-to-HTTP bridge for MCP clients that can only start a local program, such as Claude Desktop and Codex. It
forwards to the MCP server of the running Notoir and fails with *Notoir is not running* otherwise. You normally do not
type it yourself: **Tools › MCP › Copy Setup Command** writes the right configuration. See [MCP](mcp.md).
