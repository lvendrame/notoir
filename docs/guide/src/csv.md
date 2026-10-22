# CSV

Open a `.csv` or `.tsv` file and use **Edit**, **Split**, or **Preview** in the toolbar.
Preview shows a table; Split shows source text and the table together.

The separator is detected automatically. Use the separator selector to choose comma, semicolon, or tab if needed.
**First row is header** starts checked. Uncheck it to display the first row as data with numbered column headings.
Use **Columns** to hide or restore columns; **Show all** restores every column. These choices do not alter the file
and reset when it is reopened.

**Find in table** highlights matching values without hiding rows. Previous and Next navigate matches. Search ignores
case and covers visible columns in the currently filtered rows. The arrow icon beside each heading opens a column
filter and **Hide column** action. Restore a hidden column from **Columns**. Type part of a value to narrow rows, or choose **Clear**. Filters combine across columns and remain active
when a filtered column is hidden.

Double-click a data cell to edit it. Enter commits, Shift+Enter inserts a newline, and Escape cancels. Clicking away
also commits. Changes update the source text, support Undo/Redo, and use the normal Save and auto-save preferences.
CSV quoting is handled automatically, including commas and line breaks inside values.

If the source becomes invalid, the table retains its last valid contents with an error and temporarily disables cell
editing. Fix the source or undo the invalid edit. Preview is available for documents up to 20 MiB.
