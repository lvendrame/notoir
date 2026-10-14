# 06 — Event System

**Status:** Partial — typed events, outbox, frame-end drain (M1, M3); document subscribers (M5+)

## Purpose

Define typed application events: who emits them, when they are delivered, and who may subscribe — keeping a clear
ownership model instead of a global free-for-all event bus.

## Model

```rust
pub enum AppEvent {
    ApplicationStarted,
    ApplicationExiting,
    WorkspaceOpened { root: PathBuf },
    WorkspaceClosed { root: PathBuf },
    DocumentOpened { doc: DocumentId },
    DocumentChanged { doc: DocumentId, revision: u64, origin: ChangeOrigin, changes: Arc<ChangeSet> },
    DocumentSaved { doc: DocumentId, path: PathBuf },
    DocumentClosed { doc: DocumentId },
    DocumentExternallyChanged { doc: DocumentId, kind: ExternalChangeKind },
    ActiveDocumentChanged { doc: Option<DocumentId>, view: Option<ViewId> },
    EditorViewCreated { view: ViewId, doc: DocumentId, group: GroupId },
    EditorViewClosed { view: ViewId },
    LayoutChanged,
    GitStatusChanged { repo: PathBuf },
    ThemeChanged { theme: ThemeId },
    LanguageChanged { lang: LanguageCode },
    SettingsChanged { keys: Vec<SettingKey>, scope: SettingScope },
    McpServerStateChanged { running: bool },
    McpClientConnected { client: ClientId },
    McpClientDisconnected { client: ClientId },
    PluginLoaded { plugin: PluginId },
    PluginUnloaded { plugin: PluginId, reason: UnloadReason },
}

pub enum ChangeOrigin { User { view: ViewId }, Undo, Redo, Reload, Mcp(ClientId), Plugin(PluginId), Format, Internal }
```

## Delivery Rules

1. Events are **emitted** into `EventOutbox` only by domain services during command/transaction handling on the UI
   thread.
2. The outbox is **drained once at the end of each frame**, in emission order, to a fixed list of subscribers registered
   at startup:
   - `SessionTracker` (marks session state dirty for persistence),
   - `RecoveryScheduler` (schedules snapshots),
   - `AutosaveScheduler`,
   - `DerivedStateCache` (outline, breadcrumbs, preview invalidation),
   - `PluginHost` (forwards subscribed events as Lua event messages, spec 30),
   - `McpServer` (resource change notifications, activity, spec 29),
   - `GitStatusScheduler` (debounced refresh after saves).
3. Subscribers **must not mutate `AppCore` directly** while handling an event. They may only enqueue commands or start
   jobs. This prevents re-entrancy and cascades within one frame.
4. Events crossing to other threads (plugins, MCP) are converted to owned, serializable payloads; they never carry
   references into state.
5. Frequent events are coalesced before crossing threads: consecutive `DocumentChanged` for one document within a frame
   are merged; plugins receive `on_file_change` at most every `plugins.changeEventDebounceMs` (default 150 ms).
6. Events are not persisted and not replayed.

## Lua Event Mapping

| AppEvent | Lua hook |
|---|---|
| DocumentOpened | `on_file_open` |
| DocumentSaved | `on_file_save` |
| DocumentChanged (debounced) | `on_file_change` |
| DocumentClosed | `on_file_close` |
| WorkspaceOpened / Closed | `on_workspace_open` / `on_workspace_close` |
| GitStatusChanged after a commit | `on_git_commit` |
| ThemeChanged | `on_theme_change` |

## Test Cases

```gherkin
Feature: Event delivery

  Scenario: Events are delivered after the frame's commands, in order
    Given a frame in which a document is opened and then edited
    When the outbox is drained
    Then subscribers receive DocumentOpened before DocumentChanged

  Scenario: Subscribers cannot mutate state re-entrantly
    Given a subscriber that enqueues "file.save" on DocumentChanged
    When the user types a character
    Then the save runs on the next frame, not during event delivery

  Scenario: Change events are coalesced for plugins
    Given a plugin subscribed to on_file_change
    When the user types 20 characters within 100 ms
    Then the plugin receives one on_file_change call for that burst
```
