# 38 — Command-Line Interface

**Status:** Partial — M23a: `notoir --help` (translated to the OS language) and `--version` (`Notoir <version>`) print and exit without a window; unknown options and missing values print the error and a `notoir --help` hint and exit 2; `--` ends options; paths resolve against the terminal's directory (files as documents, folders as workspaces, `notoir .`), several per call, `--goto file:line[:col]` keeping colon paths whole; no paths restores the session; single-instance forwarding with `--new-window` to bypass it; `--wait` keeps the terminal until the files of that call have no tabs (the instance keeps the socket connection and writes `done`; with no instance running the command starts one and waits as a forwarder; released when the app exits). Command availability: Linux packages install `/usr/bin/notoir`; the macOS app, the AppImage and Windows installs get it from Help › Install 'notoir' Command in PATH (`/usr/local/bin` link with administrator authorization when needed, `~/.local/bin` link to the AppImage, or the install folder on the user PATH) and Help › Uninstall 'notoir' Command. Not covered by automated tests: the command on PATH after installing a real package, and `--new-window` opening a second window (needs a display); both need a manual check. Deviations: the DMG and AppImage cannot install the command or remove it on their own — the in-app commands do; `--new-window --wait` waits for that window to close

## Purpose

Define how users launch Notoir from a terminal and open files or workspaces through the `notoir` command. File,
folder, session, and single-instance behavior builds on specs 08, 23, and 31.

## Command Availability

- Every official installation format provides a `notoir` command callable from the user's `PATH` after installation,
  including the macOS DMG and Linux AppImage distributions.
- Removing Notoir removes the CLI launcher installed by that distribution.
- `notoir --help` prints the supported command syntax and exits without opening the GUI.
- `notoir --version` prints the product name and version, then exits without opening the GUI.

## Launch and Paths

```text
notoir [paths…] [--new-window] [--goto file:line[:col]] [--wait]
```

- Running `notoir` with no paths launches or focuses Notoir and restores the previous session according to spec 23.
- A path argument is resolved against the calling terminal's current working directory unless it is absolute. Files open
  as documents; folders open as workspaces. `notoir .` therefore opens the current terminal directory as the workspace
  and takes precedence over restoring the previous session.
- Multiple file and folder paths may be supplied in one invocation.
- With the default single-instance behavior, arguments are forwarded to the running instance and the new process exits.
  `--new-window` opens a separate window instead.
- `--goto file:line[:col]` opens the file and positions the caret at the one-based line and optional column. Paths
  containing colons remain valid when the suffix does not match the line/column form.
- `--wait` keeps the terminal process alive until the files opened by that invocation have no remaining tabs in Notoir.
  It works whether the request starts a new process or is forwarded to an existing instance.
- Invalid options or malformed option values print a concise error and usage hint, exit nonzero, and do not open a
  window.

## Test Cases

```gherkin
Feature: Terminal command

  Scenario: Installed command is available on PATH
    Given Notoir was installed from an official distribution
    When the user runs "notoir --version" from a terminal
    Then the command succeeds and prints the Notoir version
    And no GUI window opens

  Scenario: Help prints usage without launching the app
    When the user runs "notoir --help"
    Then the supported command syntax is printed
    And the command exits without opening a window

  Scenario: Bare command restores the prior session
    Given the previous Notoir session contains open documents
    When the user runs "notoir" with no arguments
    Then Notoir launches or focuses its window
    And the previous session is restored

  Scenario: Dot opens the terminal directory as workspace
    Given the terminal's current directory is a project folder
    When the user runs "notoir ."
    Then that project folder opens as the workspace

  Scenario: Multiple paths open from the terminal directory
    Given the terminal's current directory contains "notes.md" and a "src" folder
    When the user runs "notoir notes.md src"
    Then "notes.md" opens as a document
    And "src" opens as a workspace

  Scenario: Goto opens a file at a line and column
    When the user runs "notoir --goto src/main.rs:12:4"
    Then "src/main.rs" opens with the caret at line 12, column 4

  Scenario: New window bypasses the running instance
    Given Notoir is already running
    When the user runs "notoir --new-window ."
    Then a separate Notoir window opens with the current directory as workspace

  Scenario: Wait exits after opened tabs close
    When the user runs "notoir --wait notes.md"
    Then the terminal process remains active while "notes.md" is open in Notoir
    And it exits after the last tab for "notes.md" closes
```
