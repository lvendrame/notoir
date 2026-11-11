# 41 — Printing

**Status:** Partial — File → Print… and command registry, immutable source/selection snapshots, asynchronous native adapters, scripted app tests, macOS PDF regression and Linux container PDF smoke check; native printer/PDF dialog acceptance on Windows, macOS and Linux remains manual.

## Behavior

File → **Print…** appears in its own group after save/export/revert and before Close. It stays visible but disabled
without an active document. `file.print` is also available in the Command Palette. It has no default shortcut:
`Mod+P` remains Quick Open.

Open the system print dialog directly, without a separate Notoir preview. Print an immutable snapshot of the active
document's current buffer, including unsaved edits, without requiring or performing a save. Files, Untitled documents,
Scratchpads and read-only documents are supported. Markdown, JSON and CSV always print source text in this version.
Folding, filtering, editor zoom, theme and preview modes do not change output.

| Option | Required behavior |
|---|---|
| Destination | Installed printers and PDF destinations provided by the system |
| Content | Entire document by default; selected text available when non-empty selections exist |
| Pages | All pages or a page range |
| Copies | Copy count and collation where supported |
| Page setup | Paper size, orientation and margins where supported by the native dialog/driver |
| Printer settings | Available color, duplex and quality options supplied by the driver |

Selection printing is required on all three platforms, through native controls or an integrated accessory. Multiple
selections print in document order separated by a newline, omitting empty carets. Selection offsets are character
positions, including when text contains non-ASCII characters. The content and selection are captured at invocation;
later edits, tab changes or closing the document do not alter the job.

Use readable black monospaced text (10 pt by default), wrapping long lines to the printable width. Preserve indentation,
blank lines and Unicode with system font fallback. Normalize line endings for output and show embedded NUL as `␀`
instead of truncating the document. Exclude editor chrome, line numbers, syntax colors, headers and footers.
Use system printer defaults. Do not persist Notoir print settings.

## Integration and outcomes

All entry points use `file.print`. The app owns a replaceable `notoir_platform::printing::Printer` adapter accepting a
`PrintRequest` with title, full source, optional selected source and localized selection label. It returns `Submitted`,
`Cancelled` or an error. Core/editor crates acquire no GUI or I/O dependencies.

The native dialog owns its process's main thread. Print preparation and submission run outside the editor UI and ordered
save/settings queues. Transfer snapshot files through a private temporary directory and remove it after completion.
Do not pass document text in shell commands or log it. Prevent duplicate dialogs per window while a job is pending.

Cancel silently without submitting a job or changing document state. Preparation, dialog and submission errors produce
a localized error notification with diagnostic details and allow retry. Success says the system accepted the job,
without claiming that physical printing completed. All Notoir-owned labels and messages are translated in every locale.

Rendered output, batch printing, a dedicated PDF exporter and a separate Notoir preview are outside this version.

## Acceptance scenarios

```gherkin
Feature: Print document text
  Scenario: Print is visible and disabled without a document
    Given no document is active
    When the user opens File
    Then Print is visible and disabled
    And invoking its command does not start a print job

  Scenario: Menu and palette use the same print command
    Given an active text document
    When Print is invoked from the File menu and then from the Command Palette
    Then both invocations reach the same print adapter with the same document snapshot

  Scenario: Printing captures unsaved text without saving
    Given an Untitled document containing unsaved Unicode text, indentation and blank lines
    When the user invokes Print
    Then the adapter receives the complete current buffer
    And the document remains unsaved and dirty

  Scenario: Edits and repeated invocations do not change an open print job
    Given a print job is waiting on the native dialog
    When the user edits the document and invokes Print again
    Then the first job retains its original snapshot
    And no second dialog is opened
    And the new edits remain in the document

  Scenario: Selection printing preserves document order
    Given a document with multiple non-empty selections including Unicode text
    When the user invokes Print
    Then the adapter receives the full document and selected text as separate alternatives
    And the selections are joined in document order with a newline
    And the selection control label is localized

  Scenario: Cancellation is silent and failures allow retry
    Given an active document
    When a print operation is cancelled
    Then no completion notification is shown
    When a subsequent print operation fails
    Then a localized failure notification is shown
    When the user retries and submission succeeds
    Then a notification reports submission to the system

  Scenario: Read-only and structured documents print their source
    Given a read-only Markdown document displayed in Preview
    When the user invokes Print and cancels
    Then the adapter receives the original source text
    And read-only state, dirty state and disk contents are unchanged
    When the user prints a Scratchpad
    Then its current text reaches the adapter

  Scenario: Native print controls reflect system capabilities
    Given Notoir running on Windows, macOS or Linux
    When the user opens Print with selected text
    Then the native dialog offers entire-document and selection printing
    And printer, page range, copies and supported page setup options are available
    And system PDF destinations are available when installed

  Scenario: Native pagination preserves readable content
    Given a multi-page source containing indentation, blank lines, long lines and Unicode
    When the user prints all pages and then a page range using a system PDF destination
    Then the output has readable monochrome text in source order
    And long lines wrap within the printable area without splitting lines across page boundaries
    And the requested page range is respected
    And no editor chrome, line numbers, syntax colors, headers or footers appear
```

## Native acceptance

Run the last two scenarios on each OS with an installed PDF destination and, when available, a physical printer.
Check cancellation, unavailable printers, selected text and printer preferences, portrait/landscape and several paper
sizes. Automated adapter tests do not establish native dialog or printed-layout correctness.
