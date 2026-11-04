# 40 — Application and Window Lifecycle

**Status:** Partial — single-process host, independent editor cores, native viewport lifecycle and macOS menus, workspace deduplication, per-window restore, shared services and MCP targeting implemented. Rust regression tests and macOS native lifecycle acceptance pass. Native Windows/Linux acceptance and remaining scenario automation are pending.

## Purpose and invariants

One persistent GUI process per user/application data directory owns all native editor windows. New Window never starts another GUI process. `NOTOIR_HOME` identifies an isolated application instance. Git, platform helpers, CLI waiters and MCP stdio bridges may use transient/helper processes.

Each window owns its workspace, document registry, revisions, undo, tabs, groups, selections, layout, navigation, search, panels, dialogs, notifications, resolved settings, plugin execution, pending IO and recovery. Buffers in different windows are independent even for the same disk path; saves use spec 09 and peer windows use spec 10 conflict handling. The application shares language, user configuration writes, native menus, launch IPC, MCP transport/client registry, update coordination, font resources and the restore manifest. A crash affects all windows; recovery must preserve each independently.

## Identity and interfaces

`WindowId` is an opaque UUID persisted across restart. Native viewport IDs are derived from it, never from a PID or a document ID. Cross-window document/view references include WindowId. Local IDs remain window-local. Application requests are typed: create, activate, close, minimize, bring all forward, quit. Window commands retain the captured window/context; application commands reach the host through effects. Application events describe creation, activation, closure and quitting. Window events stay local unless explicitly published with WindowId.

The host is the only owner of the window registry, activation order and workspace-owner lookup. All mutable editor state stays on the UI thread. Workers receive immutable snapshots; replies retain their originating window, operation and workspace generation. Replies for closed/replaced contexts never modify another window.

## Native host

One hidden root eframe viewport runs the application event loop. Every editor is a peer native child viewport. The hidden root is excluded from user-facing window lists and persistence. New windows are first declared for native initialization, then rendered as immediate viewports. The wgpu renderer and immediate viewports preserve main-thread ownership; shared services are pumped once per host cycle. IPC, native menu actions and worker completions wake the host directly; a one-second fallback services configuration polling and zero-window housekeeping. Rendering namespaces widget state and applies window style without leaking into siblings. Shared fonts cover scripts used in all windows. Native focus updates activation order.

Closing any editor, including the first created, leaves all other editors alive. macOS retains the host with zero editors; New Window, Dock reopen and Open Documents work in that state. Windows/Linux exit after the last editor completes closing. Application Quit terminates the host on every platform.

## Launch routing

Parse arguments and resolve caller-relative paths before dispatch. All subsequent GUI launches forward, including `--new-window`. Canonical filesystem workspace identity has priority over new-window requests: an already-owned workspace activates its owner. A new empty window can always be created. No paths on a running host activates its last active window, or creates one if none exists. Cold launch without paths restores all prior windows when startup restoration is enabled; explicit paths do not restore unrelated windows.

Plain files go to the last active window, creating one if necessary. The first new folder uses an empty window if available, otherwise the existing replace/new-window prompt. Additional distinct folders get windows. Files accompanying folders go to the first folder's destination. `--new-window` selects a fresh destination except when workspace deduplication selects its existing owner. `--goto` applies in that destination. `--wait` tracks all documents actually opened by its invocation across destination windows and completes when all close or the host exits.

## Close and coordinated Quit

Window close checks only that window's unsaved-work policy. Flush its saves, session and recovery before dropping its services and viewport. Explicit closure removes it from the open-window restore set but preserves workspace history and recoverable hot-exit data.

Quit captures the open-window set and asks windows in deterministic order, active first. Do not destroy any window until every decision/save succeeds. A cancelled prompt or failed required save aborts Quit; completed saves remain saved. On success flush each window, commit the restore manifest, stop shared services and exit. Quit and Install uses the same flow, with one final installer handoff. The launch socket rejects new launches with a busy response while Quit is pending or committed; cancellation reopens the launch gate. Never acknowledge a launch that will be silently dropped.

## Persistence and recovery

Persist a versioned application manifest: ordered WindowIds, last active WindowId, window snapshots/geometry and recovery associations. Workspace-free windows have distinct session keys. Quit preserves its captured set; explicit close removes a window; crash restores the latest committed set. Missing paths produce notices in their owner without blocking siblings. Migrate legacy slot-zero state as one window without deleting workspace history or recovery. Recovery ownership uses session/window identity and file locks, not PID alone; centrally claim entries once. Keep metadata writes ordered and transactional. Failed migration preserves recoverable old data.

Scratchpads remain a shared library with independent per-window buffers and tabs. New windows do not globally restore Scratchpads already owned by other windows. Application language is shared; workspace theme/settings resolve locally. Shared user configuration changes broadcast. Mutating Git operations serialize by canonical repository, including nested workspaces. Plugin execution/registrations remain per-window; installation metadata is shared.

## Menus and MCP

macOS installs a single native menu bar from the command/menu definitions; no duplicate in-window bar. Windows/Linux retain in-window menus. Window navigation lists only editor windows, disambiguates identical titles, checks the active window, restores minimized selections, and offers Minimize and Bring All to Front. Menu dispatch captures its window; shortcuts execute once. With zero windows disable window commands while leaving application commands available.

One MCP listener serves all windows. `window_list` returns IDs, titles, workspace roots and active state. Context tools accept optional `window_id`; omission pins the last active open window when dequeued. Unknown explicit targets return `window_not_found`; no default target returns `no_active_window`; closure during an operation returns `window_closed`. Never retarget pending work. Trust/path/permission checks and prompts use the pinned workspace/window. Session grants belong to that context; persistent workspace grants keep their scope.

## Acceptance scenarios

```gherkin
Feature: One application with independent windows
  Scenario: New windows share one GUI process
    Given one Notoir editor window is open
    When two more windows are created
    Then all three windows belong to one GUI process
    And each has an independent workbench

  Scenario: Closing the first editor preserves its siblings
    Given three editor windows are open
    When the first-created editor is closed
    Then the other two editors remain usable
    And the application host remains running

  Scenario: An existing workspace is activated
    Given a workspace is already open in a window
    When that workspace is requested in a new window
    Then its existing window is activated
    And no duplicate workspace window is created

  Scenario: Workspace-free windows have independent sessions
    Given two windows without workspaces contain different tabs
    When their sessions are written and restored
    Then each window restores only its own tabs
    And their window identities are preserved

  Scenario: Cancelling Quit keeps all windows open
    Given multiple windows include unsaved work requiring confirmation
    When Quit is cancelled in one window
    Then no editor window is destroyed
    And the application resumes normal operation

  Scenario: macOS remains available with no editor windows
    Given Notoir is running on macOS
    When its last editor window closes
    Then the application remains running
    And New Window and Dock reopening create an editor window

  Scenario: All open windows restore after Quit
    Given workspace and workspace-free windows have distinct layouts and geometry
    When the application quits and restarts without explicit paths
    Then all windows open at Quit are restored
    And each retains its own tabs, layout and geometry

  Scenario: Window navigation activates the selected editor
    Given multiple editor windows include a minimized window
    When that window is selected from the Window menu
    Then it is restored and activated
    And window commands target it

  Scenario: MCP requests retain their target
    Given an MCP request is pinned to a window and awaits permission
    When another window becomes active
    Then permission and execution remain in the original window
    And closing the target never redirects the request

  Scenario: A peer save preserves independent dirty buffers
    Given two windows edit the same file independently
    When one window saves while the other's buffer is dirty
    Then the other window retains its unsaved text
    And it receives an external-change conflict notice
```
