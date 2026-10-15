# 37 — Editor Text References

**Status:** Not started — M17a

## Purpose

Define editor right-click behavior for selected text and copying references to file lines.

## Selection Context Menu

- Right-clicking inside an existing selection opens the editor context menu without moving the caret or changing,
  collapsing, or replacing the selection.
- When selected text belongs to a file-backed document, the context menu offers **Copy Full Reference** and
  **Copy Relative Reference**.
- Both commands copy the file path followed by a one-based line reference. A selection wholly on line 45 produces
  `path:45`; a selection spanning lines 45 through 56 produces `path:45-56`. The start and end lines are inclusive.
- **Copy Full Reference** uses the absolute file path. **Copy Relative Reference** uses the path relative to the open
  workspace root and is disabled when the file is outside that workspace. Both commands are unavailable for documents
  without a file path.

## Test Cases

```gherkin
Feature: Editor text references

  Scenario: Right-click preserves the caret and selection
    Given text is selected in a file-backed document
    And the caret is at one end of the selection
    When the user right-clicks inside the selection
    Then the editor context menu opens
    And the caret and selection remain unchanged

  Scenario: Copy full reference for one selected line
    Given text on line 45 is selected in "/myfile.md"
    When the user chooses "Copy Full Reference"
    Then the clipboard contains "/myfile.md:45"

  Scenario: Copy relative reference for selected lines
    Given text spanning lines 45 through 56 is selected in "notes/myfile.md"
    And the workspace root contains the file
    When the user chooses "Copy Relative Reference"
    Then the clipboard contains "notes/myfile.md:45-56"

  Scenario: Relative reference is unavailable outside the workspace
    Given text is selected in a file outside the open workspace
    When the user opens the editor context menu
    Then "Copy Full Reference" is available
    And "Copy Relative Reference" is disabled
```
