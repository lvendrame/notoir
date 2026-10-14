# 22 — Internationalization

**Status:** Partial — locales, plurals, detection, language switching and picker, lazy CJK fallback for editor text (M0, M3, M4); remaining namespaces arrive with their features

## Purpose

Notoir is multi-language from the first milestone, following the approach proven in `string_art_it_desktop`
(spec 24 there): embedded i18next-style JSON resources, a key-parity test across all locales, immediate language
switching, and system CJK fonts. This spec covers supported languages, detection, resource layout, translation
coverage and scope-outs, plurals, plugin strings, and testing.

Language selection is an application preference (User-scope setting `language`), never document state, never undoable.

## Supported Languages

Fourteen, identical to the reference project:

| Code | Native label |
|---|---|
| `en` (default, fallback) | English |
| `pt-BR` | Português (BR) |
| `es` | Español |
| `fr` | Français |
| `de` | Deutsch |
| `it` | Italiano |
| `nl` | Nederlands |
| `pl` | Polski |
| `ja` | 日本語 |
| `ru` | Русский |
| `uk` | Українська |
| `tr` | Türkçe |
| `zh-CN` | 简体中文 |
| `ko` | 한국어 |

RTL languages are out of scope for v1 (would need bidi in the editor and mirrored chrome).

## Detection

No saved `language` setting → OS locale via `sys-locale`, case-insensitive prefix match: `pt*` → `pt-BR`, `es*` → `es`,
`fr*` → `fr`, `de*` → `de`, `it*` → `it`, `nl*` → `nl`, `pl*` → `pl`, `ja*` → `ja`, `ru*` → `ru`, `uk*` → `uk`,
`tr*` → `tr`, `zh*` (incl. `zh-TW`/`zh-HK` until a Traditional locale exists) → `zh-CN`, `ko*` → `ko`, else `en`.
Detection is never written to settings; only an explicit choice is persisted. Implemented as a pure function
`detect_language(os_locale: &str) -> Language` with unit tests.

## Where the User Switches Language

- First-run page (spec 17), pre-selected with the detected language.
- Settings → General → Language (dropdown with native labels).
- Command Palette: "Preferences: Change Language…" (picker with native labels and English names).
- Tools → Change Language….

Switching applies immediately to the whole UI (menus, palette, panels, notifications, settings UI) without restart and
emits `LanguageChanged`.

## Resource Layout

```text
assets/locales/<lng>/<namespace>.json
```

i18next JSON (nested objects; `{{var}}` interpolation; plural suffixes `_zero/_one/_two/_few/_many/_other`). All
locales are embedded at build time (`build.rs` generates a `locales.rs` with `include_str!` per file, as in the reference
project), parsed once into a `HashMap<Language, HashMap<String /* "ns:dotted.key" */, String>>`.

| Namespace | Covers |
|---|---|
| `common` | Shared words: OK, Cancel, Close, Save, Don't Save, Retry, Show Details, Learn More, language switcher |
| `menus` | Menu bar titles and items |
| `commands` | Command titles (key = command id, e.g. `commands:file.save`) and categories |
| `toolbar` | Toolbar tooltips, contextual controls |
| `tabs` | Tab states, tab context menu specifics |
| `editor` | Find widget, go to line, info bars (large file, encoding, mixed EOL), read-only hints |
| `explorer` | Explorer panel, file operation prompts and validation |
| `panels` | Panel titles, panel menus, Outline, Search panel, Output |
| `statusbar` | Status bar items and pickers |
| `dialogs` | Save prompts, recovery prompt, confirmations, trust prompt |
| `notifications` | Toast texts |
| `settings` | Settings categories, every setting's title and description (`settings:editor.tabSize.title`) |
| `welcome` | Empty state, first-run page |
| `errors` | User-facing error messages mapped from typed errors |
| `markdown`, `json`, `diff` | Structured view UI |
| `git` | Git menu, Source Control panel, Git errors |
| `mcp` | MCP connections, permissions, activity, permission prompts |
| `plugins` | Plugin manager, permission requests, plugin errors |
| `help` | Help menu, About, keyboard shortcut reference |

Namespaces are added with the milestone that introduces the feature; every namespace exists in all 14 locales.

## API

```rust
t!("menus:file.save")                                  // &str / String
t!("notifications:saveFailed", file = name, reason = r) // interpolation
tn!("statusbar:modifiedCount", count = n)              // plural (CLDR rules)
```

- Lookup: active language → `en` → the key itself (and a `debug`-level log entry in debug builds for missing keys).
- Plural categories follow CLDR rules for each language (Polish, Russian, Ukrainian use one/few/many/other; Japanese,
  Chinese, Korean use other only). Implemented with `intl_pluralrules` or a small hand-written rules table, unit-tested
  per language.
- Errors are typed enums (`thiserror`) carrying structured data; the UI maps them to `errors:` keys with interpolation.
  Translation never depends on matching English error text.
- Keyboard shortcut rendering uses platform symbols on macOS (⌘⌥⇧⌃) and translated key names where conventional
  (e.g. "Strg" is **not** used; Notoir keeps "Ctrl" across languages like most cross-platform editors).

## Fonts for Non-Latin Scripts

- Bundled Manrope / JetBrains Mono cover Latin, Cyrillic (JetBrains Mono, Manrope Cyrillic subset verified in M0) and
  Turkish/Polish diacritics.
- For `ja`, `zh-CN`, `ko` the app loads an installed system face via `fontdb` (Hiragino / Yu Gothic / Meiryo; PingFang /
  Hiragino Sans GB / Microsoft YaHei; Apple SD Gothic Neo / Malgun Gothic; Noto Sans CJK on Linux) as a fallback after
  the UI font, only while that language is active or when CJK text appears in a document (editor fallback is always
  enabled, lazily loaded on first CJK glyph).
- A tiny bundled Noto Sans CJK subset (OFL) always renders the native names 日本語 / 简体中文 / 한국어 in the language
  pickers.

## Translation Coverage & Scope-Outs

Translated: every user-visible UI string, including menu items, command titles, settings titles/descriptions,
notifications, dialogs, tooltips, status bar items, accessibility labels, plugin permission descriptions, MCP
permission prompts and capability names.

Never translated:

- The brand name "Notoir" and the built-in theme names ("Notoir Midnight", "Notoir Paper", "Dracula").
- User content: documents, file names, paths, commit messages, plugin-supplied text without a translation.
- Log file contents (English, for supportability).
- Raw diagnostic details shown behind "Show Details" (OS error strings, Git stderr, Lua tracebacks).
- MCP tool names, schemas and descriptions sent to MCP clients (English; they are API surface for models).
- Setting keys and command ids.

## Plugin Strings

Plugins may ship `locales/<lng>.json` (single namespace per plugin, accessed as `plugin:<id>:<key>`); their manifest
titles may reference keys (`title = "%commands.count%"`). Missing translations fall back to the plugin's `en` file, then
the literal. Key parity is **not** enforced for plugins, but Plugin Developer Tools lists missing keys.

## Test Cases

```gherkin
Feature: Language selection

  Scenario: Pickers list every supported language with native names
    When the user opens the language picker
    Then "English", "Português (BR)", "Español", "Français", "Deutsch", "Italiano", "Nederlands", "Polski", "日本語", "Русский", "Українська", "Türkçe", "简体中文", and "한국어" are listed

  Scenario: Switching applies immediately
    Given the UI is in English
    When the user selects "Deutsch"
    Then the menu bar, status bar, and open Settings tab show German text without restart

  Scenario: Choice persists
    Given the user selected "Português (BR)"
    When Notoir restarts
    Then the UI is in Portuguese

  Scenario: First launch detects OS locale
    Given no language setting exists and the OS locale is "ja-JP"
    When Notoir starts
    Then the UI is in Japanese
    And the settings file does not contain a language entry

  Scenario: Unsupported locale falls back to English
    Given the OS locale is "sv-SE"
    Then the UI is in English

Feature: Plurals

  Scenario: Polish plural forms
    Given the language is Polish
    Then the modified-count label uses the "one" form for 1, "few" for 3, and "many" for 5

Feature: Key parity

  Scenario: All locales have the same keys
    Then for every namespace, every locale's leaf-key set equals English's

Feature: Typed error translation

  Scenario: Save error is translated
    Given the language is Spanish
    When a save fails with permission denied
    Then the notification text is the Spanish translation of the permission-denied save error
    And the "Show Details" content contains the raw OS error
```

## Notes

- Translation workflow: English is authored by developers; other locales are machine-assisted then reviewed. The
  key-parity test runs in CI; a missing key fails the build.
- String extraction discipline: no string literals in UI code except brand names, symbols, and debug-only text; a CI
  lint (simple grep-based test over `apps/notoir/src/**/*.rs` for `ui.label("` / `.button("` with alphabetic literals)
  catches regressions.
