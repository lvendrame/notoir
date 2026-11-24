# SVG

Open an `.svg` file normally. It starts in **Edit**, with XML syntax highlighting and the usual
editing, undo, save and recovery behavior. As with other formats, explicitly choosing a different
language mode disables the SVG-specific controls.

Use **Edit**, **Split** or **Preview** in the toolbar:

- **Edit** shows the source.
- **Split** shows source and the live SVG image together. Drag the divider to resize the panes;
  narrow panes stack the source above its preview, as with other preview formats.
- **Preview** shows the image on its own.

The image follows unsaved edits. An invalid edit shows an error while keeping the last valid
image; fix the XML or use Undo to restore the preview. The preview never writes your file.

Use the Lucide magnifiers to zoom, **expand** to fit the pane and **scan** for actual size.
The percentage input accepts 10–800%. SVG preview zoom is independent of text zoom. Scroll or
drag an enlarged image. Mode, zoom and divider size are restored with your session.

Preview resources are allocated only while a preview pane is visible. SVG source over 2 MiB
cannot be previewed. Raster output has a 4096-pixel side limit and a four-megapixel area limit;
very high zoom may therefore show an interpolated image. Source editing remains available.

SVG rendering does not run scripts or make network requests. External image files and nested
SVG data images are omitted. Bounded embedded PNG/JPEG images are supported. Text uses the
bundled font fallbacks. Ordinary `.xml` documents keep their normal text-only presentation.
