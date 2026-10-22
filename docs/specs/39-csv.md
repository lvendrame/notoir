# 39 — CSV

**Status:** Done — Edit/Split/Preview, source-preserving cell editing, header and separator choices, column visibility, find and column filters; native UI and desktop live acceptance.

## Behavior

CSV and TSV documents expose Edit / Split / Preview. Text is authoritative. Preview is a virtualized table;
Split shares the document and undo history with the text editor. Parsing runs in a background worker and only results
for the current revision and separator are accepted. Preview is disabled above 20 MiB.

Detect comma, semicolon or tab; provide an explicit separator override. First row is header defaults to true.
Unchecked, include that row as data and label headings Column 1, Column 2, etc. Empty or missing headings also use
numbered labels. Uneven records are displayed to the widest row with missing cells empty. Quoted separators,
doubled quotes, multiline fields and trailing empty fields are supported.

Columns can be hidden and restored individually or with Show All, without changing source. Search is case-insensitive
substring matching over visible columns and filtered rows: highlight matches, show their count, navigate Previous/Next.
Each heading has a filter icon opening a text-input popover with Clear and Hide column. Hidden columns can be restored from Columns. Column filters are case-insensitive substrings
combined with AND and continue applying when a column is hidden. No matches means an empty table body.

Double-click a data cell to edit; Enter commits, Shift+Enter inserts a newline, Escape cancels, leaving the field commits.
Each edit is one undo step, correctly escapes the value and preserves unrelated source and quoting. Editing a missing
cell adds only the needed separators. Filtered rows retain source indices. Read-only, stale or invalid tables cannot be
edited. Invalid CSV displays a location and the greyed last valid table until corrected. An editor opened against an
older revision cannot overwrite subsequent source changes.

Presentation choices are per-view and reset on document reopen. They never enter saved CSV content or session storage.
All actions use registered commands (`csv.showEditor`, `csv.showSplit`, `csv.showPreview`, internal `csv.action`).

## Test cases

```gherkin
Feature: CSV table
  Scenario: CSV display controls do not change source
    Given a semicolon-separated CSV document
    When the user switches between Edit, Split and Preview and toggles headers and column visibility
    Then the separator is detected and numbered headings appear without a header
    And hidden columns can be restored
    And the source text is unchanged

  Scenario: Search highlights while column filters narrow rows
    Given a CSV with several matching values
    When the user searches and combines filters on two columns
    Then search matches are case-insensitive and search alone retains all rows
    And filters combine with AND including on hidden columns
    And only matching rows remain visible

  Scenario: Cell edits preserve CSV source and support undo and save
    Given a CSV with Unicode, quoted cells, CRLF and an uneven row
    When the user changes a cell to include quotes, a delimiter and a newline
    Then unrelated source remains unchanged and the edited CSV is valid
    And undo and redo restore each version in one step
    And editing a missing cell pads the row
    And saving preserves CRLF

  Scenario: Invalid CSV retains the last valid table
    Given valid CSV displayed in Preview
    When a source edit introduces an unterminated quoted field
    Then the last valid table remains with an error
    And table edits cannot change the invalid source
    And undo restores the valid preview
```
