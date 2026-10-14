# 03 — Editor Views, Groups & Splits

## Purpose

Define `EditorView` (per-tab presentation state over a document), `EditorGroup` (a tab strip + active view), the
`LayoutTree` that arranges groups into true multi-editor splits, Duplicate View, tab drag-and-drop docking, and
Maximize Editor Group.

## Data Model

```rust
pub struct ViewId(u64);

pub struct EditorView {
    pub id: ViewId,
    pub document_id: DocumentId,
    pub kind: ViewKind,                  // Text | MarkdownPreview | MarkdownSplit | JsonTree | JsonSplit | Diff(..) | Settings
    pub selections: Selections,          // primary + secondary carets/ranges (byte offsets into rope)
    pub scroll: ScrollPosition,          // top display row + sub-row pixel offset + horizontal offset
    pub zoom: Option<f32>,               // only when editor.zoomScope = "view"
    pub folds: FoldState,                // folded ranges anchored to buffer positions
    pub minimap: MinimapViewState,       // drag state; visibility comes from settings
    pub wrap_override: Option<bool>,     // per-view word-wrap toggle (otherwise setting)
    pub pinned: bool,                    // tab pinned
    pub preview_mode: Option<PreviewSplitState>, // ratio for Editor|Preview, Text|Tree
}

pub struct GroupId(u64);

pub struct EditorGroup {
    pub id: GroupId,
    pub tabs: Vec<ViewId>,               // order = tab strip order; pinned tabs first
    pub active: Option<ViewId>,
    pub mru: Vec<ViewId>,                // activation order, for Ctrl+Tab and close fallback
}

pub enum LayoutNode {
    Group(GroupId),
    Split { axis: Axis, children: Vec<(LayoutNode, f32 /* fraction */)> }, // fractions sum to 1.0
}

pub struct LayoutTree {
    pub root: LayoutNode,
    pub groups: HashMap<GroupId, EditorGroup>,
    pub active_group: GroupId,
    pub maximized: Option<MaximizeState>, // group id + saved panel visibility
}
```

Selections store offsets anchored to the buffer; when the document changes (from any view, MCP or plugin), every
view's selections, folds and bookmarks are transformed through the transaction's change set so they stay attached to
the same text.

## Invariants

- There is always at least one group. The last group may be empty (shows the empty state, spec 17).
- A `Split` node always has ≥ 2 children; collapsing to 1 child replaces the split with that child.
- Adjacent nested splits on the same axis are flattened (`Split(H,[a, Split(H,[b,c])])` → `Split(H,[a,b,c])`).
- A view belongs to exactly one group. A document may have any number of views across groups (and within one group,
  after Duplicate View — duplicates in the same group are allowed and labelled `name.md ⧉2`).
- Minimum group size: 120 logical px along the split axis; splitter drags clamp to it.

## Split Operations

| Command | Effect |
|---|---|
| `view.splitLeft/Right/Up/Down` | Creates a new group adjacent to the active group on that side, containing a **duplicate view** of the active tab (same document, copied cursor/scroll/folds). If the active group is empty, the new group is empty. |
| `view.singleEditor` | Merges all groups into one, preserving tab order (active group's tabs first), dropping duplicate views of the same document only if they are clean duplicates (keeps one). |
| `workbench.moveTabToPreviousGroup/NextGroup` | Moves the active view; if no group exists in that direction, creates one (split right/left). |
| `workbench.focusNextGroup/PreviousGroup` | Cycles groups in layout (depth-first) order. |
| `workbench.maximizeGroup` (toggle) | Hides all other groups, sidebars and bottom panel; restores exactly (same sizes, visibility, active panel) on exit. Exits automatically if the user opens a panel or focuses a hidden group via command. |

Splitting creates an `EditorView` copy; it **never** duplicates the document or buffer.

## Duplicate View

`view.duplicate` (tab context menu "Duplicate View"): creates a new view of the same document **in the same group**,
placed after the source tab, with copied selection/scroll/folds. Edits in either are immediately visible in both; undo
history is shared (AD-3).

## Tab Drag and Drop

- Dragging a tab within a strip reorders it (pinned tabs stay in the pinned zone).
- Dragging over another group shows a docking overlay with five targets — centre (add as tab) and four edges (split):

```text
        ↑
     ←  □  →
        ↓
```

- Target highlight previews the resulting area as a translucent accent rectangle (spec 21 `accent-soft`).
- Dropping on an edge creates a new group in that direction containing the moved view.
- Dragging the last tab out of a group removes the empty group (unless it is the only group).
- Holding the platform "copy" modifier (Alt on Windows/Linux, Option on macOS) while dropping duplicates the view instead
  of moving it.
- Dragging a file from the Explorer or the OS onto a group uses the same docking targets (opens the file there).
- `Esc` during drag cancels.

## Close Behaviour

Closing a view selects the group's previous MRU view. Closing the last view of a dirty document triggers the dirty
prompt (spec 09). Closing a view whose document has other views never prompts.

## Test Cases

```gherkin
Feature: True editor splits

  Scenario: Split right creates a second group with a view of the same document
    Given "README.md" is the active tab in the only group
    When the user runs "Split Right"
    Then the layout has two groups side by side
    And the right group's active view references the same document as the left
    And the document registry still contains exactly one "README.md" document

  Scenario: Nested split layout
    Given one group with "README.md"
    When the user runs "Split Down" with "README.md" active
    And opens "notes.md" in the bottom group
    And runs "Split Right" in the bottom group with "config.json" active
    Then the layout is a vertical split whose bottom child is a horizontal split of two groups

  Scenario: Closing the last tab of a group collapses the split
    Given two groups side by side
    When the user closes every tab in the right group
    Then only one group remains and it fills the editor area

Feature: Duplicate view

  Scenario: Duplicate views share one buffer but have independent cursors
    Given "README.md" is open with the cursor on line 20
    When the user chooses "Duplicate View" and moves the new view's cursor to line 900
    Then the original view's cursor is still on line 20
    When the user types "hello" in the new view
    Then the original view shows "hello" on line 900

  Scenario: Selections follow remote edits
    Given two views of the same document, view A's cursor at the start of line 10
    When three lines are inserted at line 1 via view B
    Then view A's cursor is at the start of line 13

Feature: Tab docking

  Scenario: Dropping a tab on a group edge creates a split
    Given two tabs "a.md" and "b.md" in one group
    When the user drags "b.md" onto the bottom docking target of the same group
    Then a new group appears below containing "b.md"
    And the top group contains only "a.md"

Feature: Maximize group

  Scenario: Maximize and restore
    Given two groups, the Explorer visible at 260 px, and the bottom panel visible
    When the user toggles "Maximize Editor Group" on the left group
    Then only the left group is visible in the editor area and the sidebar and bottom panel are hidden
    When the user toggles it again
    Then both groups, the Explorer at 260 px, and the bottom panel are restored
```
