# 36 — Protected-File Saving

**Status:** Done — Save as Administrator… offered on permission-denied saves of existing files; `notoir-platform::elevation` (osascript / pkexec / UAC) writes through a temp file and one elevated copy-and-rename that keeps owner and mode; same conflict check and encoding as the save pipeline (`fs_io::save_with`); cancel and failure keep the document dirty with its recovery snapshot and offer Save As (M17a). Toggle Read-Only unlocks files opened read-only by permissions so they can be edited

## Purpose

Define how Notoir lets a user save edits back to a file that requires administrator authorization.

## Administrator Save

- If saving a file fails because the target requires elevated permissions, offer **Save as Administrator…**.
- Selecting it opens an administrator authorization dialog using the platform's supported elevation flow. Authorization
  applies only to this save attempt; Notoir does not retain elevated access.
- If authorization succeeds, retry the save to the same file path through the normal save pipeline, including its
  external-change conflict check and atomic-write rules.
- If the user cancels or authorization / the retry fails, leave the document dirty and its recovery snapshot available.
  Report the failure and keep the regular Save As option available.
- Do not offer elevation for failures unrelated to permissions, such as disk full, external-change conflicts, or an
  unsupported encoding.

## Test Cases

```gherkin
Feature: Protected-file saving

  Scenario: Administrator authorization saves the protected file
    Given a dirty document whose target requires administrator authorization
    When the user chooses "Save as Administrator…" and authorizes the attempt
    Then Notoir retries the save to the same path through the save pipeline
    And the document becomes clean when the save succeeds

  Scenario: Canceling administrator authorization preserves edits
    Given a dirty document whose target requires administrator authorization
    When the user chooses "Save as Administrator…" and cancels authorization
    Then the document remains dirty
    And its recovery snapshot remains available
    And Save As remains available

  Scenario: Administrator retry failure preserves edits
    Given a dirty document whose target requires administrator authorization
    When authorization succeeds but the save retry fails
    Then the document remains dirty
    And the user is shown the save failure
    And Save As remains available
```
