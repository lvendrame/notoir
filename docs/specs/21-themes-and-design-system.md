# 21 — Themes, Design System & Accessibility

**Status:** Partial — tokens, built-in themes, icons, contrast, theme switching incl. auto (M0, M3); custom themes (M26)

## Purpose

Define the visual language, design tokens, the three built-in themes, the theme file format, typography, iconography,
component vocabulary and accessibility rules. Normative for every UI surface.

## Design Direction

Minimal, modern, refined, dense enough for power users, native-desktop oriented, slightly premium. The chrome recedes;
text is the hero. Avoid: rounded cards, large whitespace, dashboard aesthetics, gradients, giant icons, gratuitous
animation, unnecessary separators. Elevation is expressed by background lightness steps, not shadows (shadows only for
floating overlays: palette, menus, toasts, drag previews).

The visual reference is the set of rendered app screenshots in `docs/design/*.png`, produced by
`cargo test -p notoir --test render` (output in `target/ui-snapshots`) and copied there whenever a milestone changes
the look. They show the real app rather than a separate mockup that could drift; when a screenshot and this document
disagree, this document defines the rule and the UI is fixed.

## Design Tokens

All colours come from the active theme's token table; no widget hard-codes a colour. Tokens map to `egui::Visuals`
plus a `NotoirPalette` struct for editor/syntax/git/diff colours.

### UI tokens

| Token | Usage |
|---|---|
| `bg.app` | Window background, menu bar, status bar |
| `bg.panel` | Sidebars, bottom panel, inactive tabs |
| `bg.panelRaised` | Inputs, buttons, hovered rows |
| `bg.editor` | Editor text area, active tab |
| `bg.overlay` | Palette, menus, popovers, toasts |
| `border` / `border.strong` | Hairlines / focus + hover borders |
| `text.primary` / `text.secondary` / `text.tertiary` / `text.disabled` | Text hierarchy |
| `accent` / `accent.strong` / `accent.soft` | Single brand accent: active, selected, focus ring, links / filled primary button / selected-row fill |
| `danger` / `warning` / `success` / `info` | Semantic states (never reuse accent for them) |
| `focus.ring` | 2 px keyboard focus outline (defaults to accent) |
| `shadow.float` | Overlays only |

### Editor tokens

`editor.background`, `editor.foreground`, `editor.lineHighlight`, `editor.selection`, `editor.selectionInactive`,
`editor.cursor`, `editor.whitespace`, `editor.indentGuide`, `editor.ruler`, `editor.findMatch`,
`editor.findMatchCurrent`, `editor.bracketMatch`, `gutter.background`, `gutter.lineNumber`, `gutter.lineNumberActive`,
`minimap.block`, `minimap.viewport`, `minimap.viewportHover`.

### Syntax tokens

`syntax.comment`, `keyword`, `string`, `number`, `constant`, `function`, `type`, `variable`, `property`, `operator`,
`punctuation`, `tag`, `attribute`, `regex`, `escape`, `heading`, `emphasis`, `strong`, `link`, `code`, `quote`,
`listMarker`, `invalid`. syntect scopes map onto these (e.g. `entity.name.function` → `function`,
`markup.heading` → `heading`).

### Git & diff tokens

`git.added`, `git.modified`, `git.deleted`, `git.untracked`, `git.renamed`, `git.conflict`, `git.ignored`;
`diff.insertedLine`, `diff.removedLine`, `diff.insertedText`, `diff.removedText`, `diff.gutterInserted`,
`diff.gutterRemoved`.

## Built-in Themes

Shipped as TOML in `assets/themes/` using the same format as user themes.

| Token | Notoir Midnight (dark, default) | Notoir Paper (light) | Dracula |
|---|---|---|---|
| bg.app | `#14161b` | `#ecebe6` | `#21222c` |
| bg.panel | `#181a20` | `#f3f2ee` | `#21222c` |
| bg.panelRaised | `#22252d` | `#e4e2dc` | `#343746` |
| bg.editor | `#1b1d23` | `#fbfaf7` | `#282a36` |
| bg.overlay | `#24272f` | `#ffffff` | `#343746` |
| border | `#2b2e37` | `#d9d6ce` | `#191a21` |
| border.strong | `#3a3e4a` | `#bfbbb1` | `#44475a` |
| text.primary | `#e6e7ea` | `#22231f` | `#f8f8f2` |
| text.secondary | `#a3a7b2` | `#55574f` | `#c5c8d6` |
| text.tertiary | `#6e7380` | `#85877e` | `#6272a4` |
| accent | `#7aa2ff` | `#3a5fc8` | `#bd93f9` |
| accent.strong | `#4f7be8` | `#2f4fae` | `#9b6ef3` |
| danger | `#ff6b6b` | `#c6392f` | `#ff5555` |
| warning | `#e5b454` | `#a8740d` | `#ffb86c` |
| success | `#6cc58c` | `#2f8a4b` | `#50fa7b` |
| editor.selection | `#2f4170` | `#c9d6f5` | `#44475a` |
| editor.lineHighlight | `#20232a` | `#f3f1ea` | `#2f3241` |
| editor.cursor | `#e6e7ea` | `#22231f` | `#f8f8f2` |
| syntax.comment | `#6e7380` | `#8a8c82` | `#6272a4` |
| syntax.keyword | `#c792ea` | `#8a3fb0` | `#ff79c6` |
| syntax.string | `#a5d6a7` | `#3d7a2e` | `#f1fa8c` |
| syntax.number | `#f0a875` | `#b05a12` | `#bd93f9` |
| syntax.function | `#82aaff` | `#2a5bbf` | `#50fa7b` |
| syntax.type | `#ffcb6b` | `#9a6a00` | `#8be9fd` |
| syntax.heading | `#7aa2ff` | `#2f4fae` | `#bd93f9` |
| git.added | `#6cc58c` | `#2f8a4b` | `#50fa7b` |
| git.modified | `#e5b454` | `#a8740d` | `#ffb86c` |
| git.deleted | `#ff6b6b` | `#c6392f` | `#ff5555` |
| git.untracked | `#7fd1c7` | `#1f7f74` | `#8be9fd` |
| git.conflict | `#ff7ac6` | `#b0287a` | `#ff79c6` |

Remaining tokens are derived in the theme file (e.g. `accent.soft = accent @ 14 %`). Exact values are finalized in M3
against contrast checks (below); the table is the starting palette. Dracula follows the official Dracula palette (MIT).

## Theme File Format

```toml
[meta]
id = "notoir-midnight"
name = "Notoir Midnight"
kind = "dark"            # dark | light (drives egui base visuals and image icon tint)
format = 1

[ui]
"bg.app" = "#14161b"
"accent" = "#7aa2ff"
"accent.soft" = { base = "accent", alpha = 0.14 }

[editor]
background = "#1b1d23"
foreground = "#e6e7ea"

[syntax]
keyword = "#c792ea"
string = "#a5d6a7"
comment = { color = "#6e7380", italic = true }

[git]
added = "#6cc58c"

[diff]
insertedLine = { base = "git.added", alpha = 0.12 }
```

Custom themes (Phase 6) are loaded from `<config>/themes/*.toml`; missing tokens inherit from the built-in theme of the
same `kind`. Theme switching is instant (no restart) and emits `ThemeChanged`. `workbench.theme = "auto"` follows the
OS light/dark preference (`workbench.autoThemeLight`, `workbench.autoThemeDark`).

## Typography

| Role | Font | Notes |
|---|---|---|
| UI | Manrope (400/500/600/700), bundled | 13 px base; menus 13 px; status bar 12 px |
| Editor | JetBrains Mono (400/500/700, italic), bundled; user-selectable system font | 14 px default (`editor.fontSize`), line height 1.5 |
| Numeric readouts (Ln/Col, zoom %, counts) | JetBrains Mono | tabular figures; functional cue for precise values |
| CJK UI fallback | system face loaded on demand (spec 22) | |

- Section label style (panel headers): 11 px, 700 weight, 0.06 em tracking, uppercase, `text.tertiary` — the only such
  style.
- No more than two bundled families. Emphasis via weight/colour, not more fonts.

## Spacing, Size, Shape

- Base unit 4 px; paddings 4/6/8/12/16.
- Radii: 4 px (buttons, inputs, tabs' top corners), 6 px (overlays/menus/toasts). No pill shapes except toggle chips and
  the segmented control.
- Heights: menu bar 28 px, toolbar 36 px, tab strip 34 px (`compact` 28 px), breadcrumbs 22 px, status bar 24 px, list
  rows 22 px (`compact` 20 px).
- Icons: 16 px default, 14 px compact/inline. Lucide stroke 1.75 at 16 px.
- `workbench.uiScale` (0.8–1.5) multiplies all chrome via `egui::Context::set_zoom_factor` — independent from editor
  zoom.

## Iconography

- lucide-static **1.48.0** SVGs in `assets/icons/lucide/`, rasterized with resvg at the needed physical size and tinted
  at draw time (same pipeline as string_art_it_desktop's `icons.rs`). Icon enum in `apps/notoir/src/icons.rs`; an icon
  test asserts every enum variant's SVG exists and renders.
- File-type icons: a small, consistent set derived from lucide (`file-text`, `file-json`, `file-code`, `file-image`,
  `file-terminal`, `file-cog`, `book-open` for Markdown) tinted by language category colour; folders `folder` /
  `folder-open`. No multi-colour third-party icon packs in v1.
- Never use emoji in UI chrome.
- App icon: `assets/app-icon/notoir.svg` source; generated `.icns`, `.ico`, PNG set via `examples/gen_icons.rs`.

## Component Vocabulary

Panels/sidebars, tab strip, segmented control (contextual Edit|Split|Preview), icon button (28×28 hit target,
16 px icon), toggle checkbox row, text input with inline icons (find widget), dropdown/picker, list row (tree/list),
overlay picker (palette, Quick Open), context menu, toast, info bar (in-editor banner: `bg.panelRaised`, left
semantic-colour bar, message, right-aligned text buttons), splitter (1 px line, 6 px hit area, accent on drag), docking
overlay. New screens compose these; new patterns require updating this spec.

## Motion

Only where it orients: tab insertion/drag reorder (120 ms), panel collapse (120 ms), palette fade (80 ms), smooth
scroll for jumps (`editor.smoothScrolling`, default on, 150 ms). All motion disabled when the OS reports reduced motion
or `workbench.reduceMotion = true`.

## Accessibility

- Contrast: text.primary and text.secondary ≥ 4.5:1 against their backgrounds; text.tertiary ≥ 3:1 (non-essential
  metadata only); focus ring ≥ 3:1 against adjacent colours. A unit test computes WCAG contrast for every built-in theme
  token pair listed in the test table.
- Never rely on colour alone: Git status uses letter badges, diff uses `+`/`−` gutter markers and background, tab
  states use distinct icons, errors use icons + text.
- Full keyboard operation: every panel, menu, tab and dialog reachable; visible focus ring; `F6` cycles focus regions
  (editor → sidebar → bottom panel → status bar → toolbar).
- Tooltips on all icon-only controls (label + shortcut).
- AccessKit enabled (egui's AccessKit integration); editor exposes its visible text region and caret; screen-reader
  testing on VoiceOver and NVDA before 1.0.
- Configurable UI scale and editor font size.

## Test Cases

```gherkin
Feature: Themes

  Scenario: Switch theme instantly
    Given the theme is Notoir Midnight
    When the user selects Notoir Paper in Settings
    Then all chrome, editor, syntax, and Git colours switch without restart

  Scenario: Built-in themes meet contrast
    Then every built-in theme's primary and secondary text tokens have contrast ≥ 4.5:1 against bg.editor and bg.panel

  Scenario: Custom theme inherits missing tokens
    Given a custom dark theme defining only bg.editor
    When it is selected
    Then all other tokens come from Notoir Midnight

  Scenario: Auto theme follows OS
    Given workbench.theme is "auto"
    When the OS switches to light mode
    Then Notoir Paper is applied

Feature: Icons

  Scenario: Every icon asset exists
    Then every Icon enum variant has an SVG file that rasterizes without error
```
