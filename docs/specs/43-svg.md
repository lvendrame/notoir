# 43 — SVG Editing and Preview

**Status:** Done — editable XML source with Edit/Split/Preview, asynchronous live rendering, separate preview zoom, bounded lazy resources and session restoration

SVG files remain ordinary text documents with XML syntax highlighting, undo, dirty state,
external-change handling and the normal save pipeline. `.svg` detection is case-insensitive;
plain XML files do not acquire SVG controls. An explicit language override away from XML disables SVG controls. Edit is the default mode.

The existing toolbar's Edit / Split / Preview selector dispatches `svg.showEditor`,
`svg.showSplit` and `svg.showPreview`. Split uses the same adaptive orientation, draggable
divider and 20–80% ratio bounds as other source/preview formats. Each view keeps its own mode.
Preview renders the current unsaved source; switching modes never changes file content.

SVG zoom is separate from text-editor zoom. Lucide zoom-in, zoom-out, expand (Fit to pane),
and scan (Actual size) controls accompany a 10–800% input. Fit preserves aspect ratio;
100% uses the SVG's natural logical size. Enlarged previews scroll/pan. Raster output is
capped to 4096 per side and 4 megapixels, so extreme zoom can interpolate a bounded texture.

Rendering uses the existing resvg dependency on a dedicated background queue. Source is capped
at 2 MiB; raster/upload/texture reservations share a 96 MiB window budget. Requests use cheap
rope snapshots, are debounced 100 ms, and allow at most one pending job per view. Superseded
requests are cancelled and revision/zoom checks discard stale results. Rendering resources are
created only for visible Split/Preview panes; source-only views allocate no worker or texture.
Hidden/closed panes cancel pending work and release cached textures. View preferences remain.

Invalid SVG displays a translated error and retains the last valid preview while the source
can be corrected or undone. Rendering cannot change source bytes. Scripts and network requests
are not executed. External image paths and nested SVG data images are omitted; bounded embedded
PNG/JPEG images are supported. Text uses bundled Manrope/JetBrains Mono font fallbacks.

Session v1 gains optional `svg_mode`, `svg_zoom` and `svg_ratio` fields. Existing sessions still
load in Edit. Restore preserves SVG mode, independent text/preview zoom and divider ratio.

## Scenarios

```gherkin
Feature: SVG presentation
  Scenario: SVG uses Edit Split and Preview without changing source
    Given a local SVG document
    When it is opened and the user switches between Split, Preview and Edit
    Then the active view changes to the corresponding presentation
    And its source remains editable, clean and unchanged
    And source-only presentation releases preview resources

  Scenario: SVG preview follows unsaved changes and recovers from invalid source
    Given an SVG in Split mode with a rendered preview
    When its source is edited without saving
    Then the preview catches up to the document revision
    And its file on disk remains unchanged
    When invalid SVG is entered
    Then the preview reports an error and keeps the last valid image
    When the invalid edit is undone
    Then the error clears and the preview matches the current revision

  Scenario: SVG presentation survives session restoration
    Given an SVG in Split mode with independent text and preview zoom and a resized divider
    When its session is captured, closed and restored
    Then its Split mode, both zoom values and divider ratio are restored
    And returning to Edit releases preview resources

Feature: SVG resource limits
  Scenario: SVG rasterization preserves transparency and bounds allocation
    Given an SVG with a translucent red rectangle
    When it is rasterized
    Then its natural dimensions and premultiplied alpha are preserved
    And releasing the raster releases its reservation
    And very large natural dimensions rasterize within the viewport bounds
    And excessive source size or an exhausted budget rejects the preview
```
