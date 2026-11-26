//! SVG remains an editable XML document. Preview workers rasterize revisioned
//! rope snapshots; no decoder or texture is allocated for source-only views.
use crate::{
    core::AppCore,
    io::{IoQueue, IoResult},
};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::view::{SvgPresentation, ViewKind};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: f32 = 4_194_304.0;
const BUDGET: usize = 96 * 1024 * 1024;
const DEBOUNCE_MS: u64 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderKey {
    pub doc: DocumentId,
    pub revision: u64,
    viewport: [u32; 2],
    zoom: Option<u32>,
    density: u32,
}
#[derive(Debug)]
struct Reservation {
    used: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
#[derive(Debug)]
pub struct Raster {
    pub size: [usize; 2],
    pub natural: [f32; 2],
    pub rgba: Vec<u8>,
    reservation: Reservation,
}
#[derive(Debug)]
pub struct Rendered {
    pub view: ViewId,
    pub key: RenderKey,
    pub result: Result<Raster, String>,
}
pub struct Preview {
    pub texture: Option<egui::TextureHandle>,
    pub natural: [f32; 2],
    pub error: Option<String>,
    pub revision: Option<u64>,
    pub ready: Option<Raster>,
    pub effective_zoom: f32,
    key: Option<RenderKey>,
    pending: Option<RenderKey>,
    cancel: Option<Arc<AtomicBool>>,
    at_ms: u64,
    allocation: Option<Reservation>,
}
impl Default for Preview {
    fn default() -> Self {
        Self {
            texture: None,
            natural: [0.0; 2],
            error: None,
            revision: None,
            ready: None,
            effective_zoom: 100.0,
            key: None,
            pending: None,
            cancel: None,
            at_ms: 0,
            allocation: None,
        }
    }
}
impl Preview {
    pub fn accept_raster(&mut self, raster: Raster) {
        self.allocation = Some(raster.reservation);
        self.revision = self.key.as_ref().map(|key| key.revision);
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}
pub struct SvgState {
    pub views: HashMap<ViewId, Preview>,
    pub io: IoQueue,
    used: Arc<AtomicUsize>,
    fonts: Arc<OnceLock<Arc<resvg::usvg::fontdb::Database>>>,
}
impl SvgState {
    fn new(poster: crate::io::Poster) -> Self {
        Self {
            views: HashMap::new(),
            io: IoQueue::start(move || poster.wake()),
            used: Arc::new(AtomicUsize::new(0)),
            fonts: Arc::new(OnceLock::new()),
        }
    }
}
fn fonts() -> Arc<resvg::usvg::fontdb::Database> {
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../assets/fonts/ui/Manrope-Regular.ttf").to_vec());
    db.load_font_data(
        include_bytes!("../../../assets/fonts/ui/JetBrainsMono-Regular.ttf").to_vec(),
    );
    db.set_serif_family("Manrope");
    db.set_sans_serif_family("Manrope");
    db.set_monospace_family("JetBrains Mono");
    Arc::new(db)
}
fn render(
    source: &str,
    key: &RenderKey,
    used: &Arc<AtomicUsize>,
    fonts: Arc<resvg::usvg::fontdb::Database>,
    cancel: &AtomicBool,
) -> Result<Raster, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("svg:tooLarge".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("svg:cancelled".into());
    }
    let mut options = resvg::usvg::Options {
        font_family: "Manrope".into(),
        fontdb: fonts,
        ..Default::default()
    };
    // Rendering source does not read arbitrary paths or start network requests.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let default_data = resvg::usvg::ImageHrefResolver::default_data_resolver();
    options.image_href_resolver.resolve_data = Box::new(move |mime, data, opts| {
        if data.len() > 1024 * 1024 || !matches!(mime, "image/png" | "image/jpeg" | "image/jpg") {
            return None;
        }
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(&*data))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS as u64 {
            return None;
        }
        default_data(mime, data, opts)
    });
    let tree = resvg::usvg::Tree::from_str(source, &options).map_err(|e| e.to_string())?;
    let natural = [tree.size().width(), tree.size().height()];
    if natural
        .iter()
        .any(|n| !n.is_finite() || *n <= 0.0 || *n > 1_000_000.0)
    {
        return Err("svg:tooLarge".into());
    }
    let density = f32::from_bits(key.density);
    let scale = key.zoom.map_or_else(
        || {
            (key.viewport[0] as f32 / natural[0])
                .min(key.viewport[1] as f32 / natural[1])
                .min(density)
        },
        |z| f32::from_bits(z) / 100.0 * density,
    );
    let mut w = (natural[0] * scale).max(1.0);
    let mut h = (natural[1] * scale).max(1.0);
    let limit = (MAX_SIDE as f32 / w)
        .min(MAX_SIDE as f32 / h)
        .min((MAX_PIXELS / (w * h)).sqrt())
        .min(1.0);
    w = (w * limit).floor().max(1.0);
    h = (h * limit).floor().max(1.0);
    let bytes = w as usize * h as usize * 4 * 3; // raster, upload and texture copies
    used.try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
        n.checked_add(bytes).filter(|n| *n <= BUDGET)
    })
    .map_err(|_| "svg:memoryLimit".to_owned())?;
    let reservation = Reservation {
        used: used.clone(),
        bytes,
    };
    if cancel.load(Ordering::Acquire) {
        return Err("svg:cancelled".into());
    }
    let mut pixels = resvg::tiny_skia::Pixmap::new(w as u32, h as u32)
        .ok_or_else(|| "svg:tooLarge".to_owned())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w / natural[0], h / natural[1]),
        &mut pixels.as_mut(),
    );
    if cancel.load(Ordering::Acquire) {
        return Err("svg:cancelled".into());
    }
    Ok(Raster {
        size: [w as usize, h as usize],
        natural,
        rgba: pixels.take(),
        reservation,
    })
}
impl AppCore {
    pub fn is_svg(&self, doc: DocumentId) -> bool {
        self.workbench.document(doc).is_some_and(|d| {
            d.buffer.as_text().is_some()
                && d.language.as_str() == "xml"
                && d.path()
                    .map(|p| p.as_os_str())
                    .or_else(|| d.suggested_name.as_deref().map(std::ffi::OsStr::new))
                    .is_some_and(|p| {
                        std::path::Path::new(p)
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
                    })
        })
    }
    pub fn set_svg_mode(&mut self, view: ViewId, mode: &str) {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return;
        };
        if !self.is_svg(doc) {
            return;
        }
        if let Some(v) = self.workbench.views.get_mut(view) {
            v.kind = match mode {
                "split" => ViewKind::SvgSplit,
                "preview" => ViewKind::SvgPreview,
                _ => ViewKind::Text,
            };
            if v.kind != ViewKind::Text {
                v.svg
                    .get_or_insert_with(|| Box::new(SvgPresentation::default()));
            }
            if v.kind != ViewKind::SvgPreview {
                self.effects.push(crate::core::Effect::FocusEditor(view));
            }
        }
    }
    /// Cancel hidden/closed previews, retaining only lightweight view preferences.
    pub fn tick_svg(&mut self) {
        let visible: HashSet<_> = self
            .workbench
            .layout
            .groups_in_order()
            .into_iter()
            .filter_map(|g| self.workbench.layout.group(g)?.active)
            .filter(|v| {
                self.workbench.view(*v).is_some_and(|v| {
                    matches!(v.kind, ViewKind::SvgPreview | ViewKind::SvgSplit)
                        && self.is_svg(v.document_id)
                })
            })
            .collect();
        if visible.is_empty() {
            self.svg = None;
        } else if let Some(state) = &mut self.svg {
            state.views.retain(|v, _| visible.contains(v));
        }
    }
    pub fn prepare_svg(&mut self, ctx: &egui::Context, view: ViewId, size: egui::Vec2) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(d) = self.workbench.document(v.document_id) else {
            return;
        };
        if !self.is_svg(d.id) || !matches!(v.kind, ViewKind::SvgSplit | ViewKind::SvgPreview) {
            return;
        }
        let density = ctx.pixels_per_point();
        let key = RenderKey {
            doc: d.id,
            revision: d.revision,
            viewport: [
                (size.x * density).clamp(1.0, 16384.0) as u32,
                (size.y * density).clamp(1.0, 16384.0) as u32,
            ],
            zoom: v.svg.as_ref().and_then(|v| v.zoom).map(f32::to_bits),
            density: density.to_bits(),
        };
        let poster = self.io.poster();
        let state = self
            .svg
            .get_or_insert_with(|| Box::new(SvgState::new(poster)));
        let preview = state.views.entry(view).or_default();
        if preview.key.as_ref() == Some(&key) {
            return;
        }
        if let Some(pending) = &preview.pending {
            if pending != &key
                && let Some(cancel) = &preview.cancel
            {
                cancel.store(true, Ordering::Release);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(DEBOUNCE_MS));
            return;
        }
        if preview.key.is_some() && self.now_ms.saturating_sub(preview.at_ms) < DEBOUNCE_MS {
            ctx.request_repaint_after(std::time::Duration::from_millis(DEBOUNCE_MS));
            return;
        }
        if d.buffer.rope().len_bytes() > MAX_SOURCE_BYTES {
            preview.error = Some("svg:tooLarge".into());
            preview.key = Some(key);
            return;
        }
        let rope = d.buffer.rope().clone();
        let cancel = Arc::new(AtomicBool::new(false));
        preview.cancel = Some(cancel.clone());
        preview.pending = Some(key.clone());
        preview.at_ms = self.now_ms;
        let used = state.used.clone();
        let fonts = state.fonts.clone();
        state.io.submit(move || {
            let result = if cancel.load(Ordering::Acquire) {
                Err("svg:cancelled".into())
            } else {
                let fonts = fonts.get_or_init(crate::svg::fonts).clone();
                render(&rope.to_string(), &key, &used, fonts, &cancel)
            };
            Some(IoResult::SvgRendered(Box::new(Rendered {
                view,
                key,
                result,
            })))
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(DEBOUNCE_MS));
    }
    pub fn svg_rendered(&mut self, done: Rendered) {
        let Some(state) = &mut self.svg else {
            return;
        };
        let Some(preview) = state.views.get_mut(&done.view) else {
            return;
        };
        if preview.pending.as_ref() != Some(&done.key) {
            return;
        }
        preview.pending = None;
        preview.cancel = None;
        let Some(v) = self.workbench.view(done.view) else {
            return;
        };
        if !matches!(v.kind, ViewKind::SvgPreview | ViewKind::SvgSplit)
            || self
                .workbench
                .document(done.key.doc)
                .is_none_or(|d| d.revision != done.key.revision)
            || v.svg.as_ref().and_then(|v| v.zoom).map(f32::to_bits) != done.key.zoom
        {
            return;
        }
        if done.result.as_ref().is_err_and(|e| e == "svg:cancelled") {
            return;
        }
        preview.key = Some(done.key);
        match done.result {
            Ok(raster) => {
                // Texture uploads are deferred to rendering, where the egui context exists.
                preview.ready = Some(raster);
            }
            Err(e) => preview.error = Some(e),
        }
    }
    pub fn svg_zoom_action(&mut self, inv: &notoir_core::command::CommandInvocation) {
        let Some(view) = inv
            .args
            .get("view")
            .and_then(|v| v.as_u64())
            .map(ViewId)
            .or_else(|| self.workbench.active_view())
        else {
            return;
        };
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        if !self.is_svg(v.document_id) {
            return;
        }
        let command = if inv.id.as_str() == "svg.action" {
            inv.args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        } else {
            inv.id.as_str()
        };
        if command == "svg.retry" {
            if let Some(preview) = self.svg.as_mut().and_then(|s| s.views.get_mut(&view)) {
                preview.key = None;
                preview.error = None;
                if let Some(cancel) = &preview.cancel {
                    cancel.store(true, Ordering::Release);
                }
            }
            return;
        }
        let effective = self
            .svg
            .as_ref()
            .and_then(|s| s.views.get(&view))
            .map_or(100.0, |p| p.effective_zoom);
        let v = self.workbench.views.get_mut(view).unwrap();
        let options = v
            .svg
            .get_or_insert_with(|| Box::new(SvgPresentation::default()));
        options.zoom = match command {
            "svg.fit" => None,
            "svg.actualSize" => Some(100.0),
            "svg.zoomIn" => Some((options.zoom.unwrap_or(effective) * 1.25).clamp(10.0, 800.0)),
            "svg.zoomOut" => Some((options.zoom.unwrap_or(effective) / 1.25).clamp(10.0, 800.0)),
            "svg.setZoom" => inv
                .args
                .get("value")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(10.0, 800.0) as f32),
            _ => return,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> RenderKey {
        RenderKey {
            doc: DocumentId(1),
            revision: 0,
            viewport: [320, 240],
            zoom: None,
            density: 1.0_f32.to_bits(),
        }
    }
    // spec 43: "SVG rasterization preserves transparency and bounds allocation"
    #[test]
    fn raster_transparency_and_limits() {
        let used = Arc::new(AtomicUsize::new(0));
        let cancel = AtomicBool::new(false);
        let image=render(r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000" opacity="0.5"/></svg>"##,&key(),&used,fonts(),&cancel).unwrap();
        assert_eq!(image.size, [20, 10]);
        assert_eq!(image.natural, [20.0, 10.0]);
        assert_eq!(&image.rgba[..4], &[128, 0, 0, 128]);
        drop(image);
        assert_eq!(used.load(Ordering::Acquire), 0);
        let huge = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1000000" height="1000000"/>"#,
            &key(),
            &used,
            fonts(),
            &cancel,
        )
        .unwrap();
        assert_eq!(huge.size, [240, 240]);
        drop(huge);
        assert!(
            render(
                &" ".repeat(MAX_SOURCE_BYTES + 1),
                &key(),
                &used,
                fonts(),
                &cancel
            )
            .is_err()
        );
        used.store(BUDGET, Ordering::Release);
        assert!(
            render(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"/>"#,
                &key(),
                &used,
                fonts(),
                &cancel
            )
            .is_err()
        );
    }
}
