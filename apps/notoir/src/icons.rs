//! Icons (docs/specs/21-themes-and-design-system.md §Iconography):
//! lucide-static v1.48.0 SVGs rasterized on demand with resvg as
//! white-on-transparent textures and tinted at draw time, mirroring
//! `stroke="currentColor"`. Add an icon by copying its SVG from lucide-static
//! 1.48.0 into `assets/icons/lucide/` and adding a row below.

use std::collections::HashMap;

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

/// Lucide's stroke at the 16 px default size (spec 21).
pub const STROKE: f32 = 1.75;

macro_rules! icons {
    ($($variant:ident => $path:literal),* $(,)?) => {
        /// Every icon the UI uses.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon { $($variant),* }

        impl Icon {
            pub const ALL: &'static [Icon] = &[$(Icon::$variant),*];

            fn svg(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/", $path, ".svg"
                    ))),*
                }
            }
        }
    };
}

icons! {
    Bell => "lucide/bell",
    BookOpen => "lucide/book-open",
    Bookmark => "lucide/bookmark",
    Bot => "lucide/bot",
    Check => "lucide/check",
    ChevronDown => "lucide/chevron-down",
    ChevronLeft => "lucide/chevron-left",
    ChevronRight => "lucide/chevron-right",
    ChevronUp => "lucide/chevron-up",
    ChevronsDownUp => "lucide/chevrons-down-up",
    CircleAlert => "lucide/circle-alert",
    CircleCheck => "lucide/circle-check",
    CircleQuestionMark => "lucide/circle-question-mark",
    Columns2 => "lucide/columns-2",
    Copy => "lucide/copy",
    Download => "lucide/download",
    Ellipsis => "lucide/ellipsis",
    File => "lucide/file",
    FileCode => "lucide/file-code",
    FileCog => "lucide/file-cog",
    FileImage => "lucide/file-image",
    FileJson => "lucide/file-json",
    FilePlus => "lucide/file-plus",
    FileTerminal => "lucide/file-terminal",
    FileText => "lucide/file-text",
    FileX => "lucide/file-x",
    Files => "lucide/files",
    Folder => "lucide/folder",
    FolderOpen => "lucide/folder-open",
    FolderPlus => "lucide/folder-plus",
    GitBranch => "lucide/git-branch",
    GitCompare => "lucide/git-compare",
    GitMerge => "lucide/git-merge",
    Globe => "lucide/globe",
    Info => "lucide/info",
    Keyboard => "lucide/keyboard",
    ListTree => "lucide/list-tree",
    Lock => "lucide/lock",
    LockOpen => "lucide/lock-open",
    Maximize => "lucide/maximize",
    Minus => "lucide/minus",
    PanelBottom => "lucide/panel-bottom",
    PanelLeft => "lucide/panel-left",
    PanelRight => "lucide/panel-right",
    Pin => "lucide/pin",
    PinOff => "lucide/pin-off",
    Plus => "lucide/plus",
    Puzzle => "lucide/puzzle",
    Redo2 => "lucide/redo-2",
    RefreshCw => "lucide/refresh-cw",
    Rows2 => "lucide/rows-2",
    Save => "lucide/save",
    Scissors => "lucide/scissors",
    Search => "lucide/search",
    Settings => "lucide/settings",
    ShieldAlert => "lucide/shield-alert",
    Trash => "lucide/trash",
    TriangleAlert => "lucide/triangle-alert",
    Undo2 => "lucide/undo-2",
    X => "lucide/x",
    ZoomIn => "lucide/zoom-in",
    ZoomOut => "lucide/zoom-out",
    AppLogo => "../app-icon/notoir",
}

/// Rasterizes an SVG at `px` square pixels. `stroke_width` overrides the
/// lucide source value of 2.
/// `currentColor` becomes white so the texture can be tinted.
pub fn rasterize(svg: &[u8], px: u32, stroke_width: Option<f32>) -> Option<ColorImage> {
    let mut text = String::from_utf8_lossy(svg).replace("currentColor", "#ffffff");
    if let Some(w) = stroke_width {
        text = text.replace("stroke-width=\"2\"", &format!("stroke-width=\"{w}\""));
    }
    let tree = resvg::usvg::Tree::from_str(&text, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(ColorImage::from_rgba_premultiplied(
        [px as usize, px as usize],
        pixmap.data(),
    ))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    icon: Icon,
    px: u32,
    stroke_milli: u32,
}

/// Texture cache keyed by icon, physical pixel size and stroke width.
#[derive(Default)]
pub struct IconCache {
    textures: HashMap<Key, TextureHandle>,
}

impl IconCache {
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        icon: Icon,
        size: f32,
        stroke_width: Option<f32>,
    ) -> TextureHandle {
        let px = (size * ctx.pixels_per_point()).ceil().max(1.0) as u32;
        let key = Key {
            icon,
            px,
            stroke_milli: (stroke_width.unwrap_or(STROKE) * 1000.0) as u32,
        };
        self.textures
            .entry(key)
            .or_insert_with(|| {
                let image = rasterize(icon.svg(), px, stroke_width)
                    .unwrap_or_else(|| ColorImage::new([1, 1], vec![Color32::TRANSPARENT]));
                ctx.load_texture(format!("icon-{icon:?}-{px}"), image, TextureOptions::LINEAR)
            })
            .clone()
    }

    /// An `egui::Image` for `icon` at `size` points, tinted `color`.
    pub fn image(
        &mut self,
        ctx: &egui::Context,
        icon: Icon,
        size: f32,
        color: Color32,
    ) -> egui::Image<'static> {
        let tex = self.texture(ctx, icon, size, Some(STROKE));
        egui::Image::new((tex.id(), egui::vec2(size, size))).tint(color)
    }
}

/// Window icon from `assets/app-icon/notoir.svg`.
pub fn app_icon() -> egui::IconData {
    let image = rasterize(Icon::AppLogo.svg(), 256, None).expect("app icon rasterizes");
    egui::IconData {
        rgba: image
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect(),
        width: 256,
        height: 256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_with_visible_pixels() {
        for &icon in Icon::ALL {
            let img = rasterize(icon.svg(), 32, None).unwrap_or_else(|| panic!("{icon:?}"));
            assert!(
                img.pixels.iter().any(|p| p.a() > 0),
                "{icon:?} rendered empty"
            );
        }
    }
}
