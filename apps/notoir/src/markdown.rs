//! Markdown modes, preview cache, links, images and editing aids
//! (docs/specs/24). Rendering lives in `shell/md_preview.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::view::ViewKind;
use notoir_editor::markdown::{self, MdDoc};
use notoir_editor::{ChangeSet, EditKind, Range, Selection};

use crate::core::AppCore;
use crate::files::OpenAt;
use crate::io::IoResult;

/// Re-parse at most this often while typing (spec 24: 100 ms).
pub const PARSE_DEBOUNCE_MS: u64 = 100;
/// Larger documents parse less often.
pub const PARSE_DEBOUNCE_LARGE_MS: u64 = 250;
const LARGE_DOC_CHARS: usize = 200_000;
/// Decoded images are scaled down to this width.
pub const IMAGE_MAX_WIDTH: u32 = 1600;

/// A parsed document for the preview.
pub struct Parsed {
    pub revision: u64,
    pub doc: Arc<MdDoc>,
    pub at_ms: u64,
}

/// Decoded local image (or why it could not be shown).
#[derive(Debug)]
pub enum ImageData {
    Ready {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    Missing(String),
}

#[derive(Default)]
pub struct MarkdownState {
    pub parsed: HashMap<DocumentId, Parsed>,
    /// Images requested from disk (path → mtime when requested).
    pub requested: HashMap<PathBuf, u64>,
    /// Decoded images waiting for the UI to upload them as textures.
    pub decoded: Vec<(PathBuf, ImageData)>,
}

fn epoch_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// Decodes a raster or SVG image, scaled to [`IMAGE_MAX_WIDTH`].
pub fn decode_image(path: &Path) -> ImageData {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return ImageData::Missing(e.to_string()),
    };
    let is_svg = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if is_svg {
        let opt = resvg::usvg::Options::default();
        let tree = match resvg::usvg::Tree::from_data(&bytes, &opt) {
            Ok(t) => t,
            Err(e) => return ImageData::Missing(e.to_string()),
        };
        let size = tree.size();
        let scale = (IMAGE_MAX_WIDTH as f32 / size.width()).min(2.0);
        let (w, h) = (
            (size.width() * scale).ceil() as u32,
            (size.height() * scale).ceil() as u32,
        );
        let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)) else {
            return ImageData::Missing("empty image".into());
        };
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        // tiny-skia pixels are premultiplied; egui expects unmultiplied.
        let rgba = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        return ImageData::Ready {
            width: w as usize,
            height: h as usize,
            rgba,
        };
    }
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            let img = if img.width() > IMAGE_MAX_WIDTH {
                img.resize(
                    IMAGE_MAX_WIDTH,
                    u32::MAX,
                    image::imageops::FilterType::Triangle,
                )
            } else {
                img
            };
            let rgba = img.to_rgba8();
            ImageData::Ready {
                width: rgba.width() as usize,
                height: rgba.height() as usize,
                rgba: rgba.into_raw(),
            }
        }
        Err(e) => ImageData::Missing(e.to_string()),
    }
}

impl AppCore {
    /// Is the document Markdown (contextual controls, aids)?
    pub fn is_markdown(&self, doc: DocumentId) -> bool {
        self.workbench
            .document(doc)
            .is_some_and(|d| d.language.as_str() == "markdown")
    }

    /// Sets a view's display mode (`edit` / `split` / `preview`).
    pub fn set_markdown_mode(&mut self, view: ViewId, mode: &str) {
        let kind = match mode {
            "split" => ViewKind::MarkdownSplit,
            "preview" => ViewKind::MarkdownPreview,
            _ => ViewKind::Text,
        };
        if let Some(v) = self.workbench.views.get_mut(view) {
            v.kind = kind;
        }
        if kind != ViewKind::MarkdownPreview {
            self.effects.push(crate::core::Effect::FocusEditor(view));
        }
    }

    /// New Markdown views start in `markdown.defaultMode`.
    pub fn apply_markdown_default_mode(&mut self, view: ViewId) {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return;
        };
        if !self.is_markdown(doc) {
            return;
        }
        let mode = self
            .settings
            .str("markdown.defaultMode", Some("markdown"))
            .to_owned();
        if mode != "edit" {
            self.set_markdown_mode(view, &mode);
        }
    }

    /// The parsed document for a preview, re-parsed after the debounce.
    /// Returns it and whether a newer parse is pending.
    pub fn markdown_doc(&mut self, doc: DocumentId) -> Option<(Arc<MdDoc>, bool)> {
        let d = self.workbench.document(doc)?;
        let revision = d.revision;
        let large = d.buffer.len_chars() > LARGE_DOC_CHARS;
        let debounce = if large {
            PARSE_DEBOUNCE_LARGE_MS
        } else {
            PARSE_DEBOUNCE_MS
        };
        let now = self.now_ms;
        let stale = match self.markdown.parsed.get(&doc) {
            Some(p) if p.revision == revision => return Some((p.doc.clone(), false)),
            Some(p) => now.saturating_sub(p.at_ms) < debounce,
            None => false,
        };
        if stale {
            return self
                .markdown
                .parsed
                .get(&doc)
                .map(|p| (p.doc.clone(), true));
        }
        let text = d.buffer.text();
        let parsed = Arc::new(markdown::parse(&text));
        self.markdown.parsed.insert(
            doc,
            Parsed {
                revision,
                doc: parsed.clone(),
                at_ms: now,
            },
        );
        Some((parsed, false))
    }

    /// Requests a local image for the preview (decoded off the UI thread).
    pub fn request_image(&mut self, path: &Path) {
        let mtime = epoch_mtime(path);
        if self.markdown.requested.get(path) == Some(&mtime) {
            return;
        }
        self.markdown.requested.insert(path.to_path_buf(), mtime);
        let p = path.to_path_buf();
        self.fs.submit(move || {
            let data = decode_image(&p);
            Some(IoResult::ImageDecoded(Box::new((p, data))))
        });
    }

    /// Resolves a Markdown link target relative to the document.
    pub fn resolve_md_target(&self, doc: DocumentId, target: &str) -> Option<PathBuf> {
        let base = self
            .workbench
            .document(doc)?
            .path()?
            .parent()?
            .to_path_buf();
        let p = Path::new(target);
        Some(if p.is_absolute() {
            p.to_path_buf()
        } else {
            base.join(p)
        })
    }

    /// A clicked link: web/mail to the OS, relative files open in Notoir
    /// (with `#anchor`), in-document anchors scroll, other schemes ask first.
    pub fn follow_md_link(&mut self, doc: DocumentId, url: &str) {
        if let Some(target) = url.strip_prefix("wiki:") {
            self.notes_follow(doc, target);
            return;
        }
        let lower = url.to_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:")
        {
            self.files.dialogs.open_external(url);
            return;
        }
        if let Some(slug) = url.strip_prefix('#') {
            let view = self.workbench.views.of_document(doc).next().map(|v| v.id);
            if let Some(view) = view {
                let at = OpenAt {
                    anchor: Some(slug.to_owned()),
                    ..Default::default()
                };
                self.place_caret_at(view, &at);
            }
            return;
        }
        if url.contains("://") || lower.starts_with("file:") {
            self.files
                .prompts
                .push_back(crate::files::Prompt::OpenLink {
                    url: url.to_owned(),
                });
            return;
        }
        let (path, anchor) = match url.split_once('#') {
            Some((p, a)) => (p, Some(a.to_owned())),
            None => (url, None),
        };
        let decoded = path.replace("%20", " ");
        match self.resolve_md_target(doc, &decoded) {
            Some(p) if p.exists() => self.open_path(
                &p,
                OpenAt {
                    anchor,
                    ..Default::default()
                },
            ),
            _ => {
                let now = self.now_ms;
                self.notifications.notify(
                    Notification::new(
                        Severity::Warning,
                        Translatable::new("notifications:linkMissing").arg("target", url),
                    ),
                    now,
                );
            }
        }
    }

    /// Preview checkbox: `[ ]` ↔ `[x]` in the source as one undoable edit.
    pub fn toggle_md_task(&mut self, doc: DocumentId, range: (usize, usize)) {
        let Some(view) = self.workbench.views.of_document(doc).next().map(|v| v.id) else {
            return;
        };
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        if range.1 > d.buffer.len_chars() || d.is_read_only() {
            return;
        }
        let current: String = d.buffer.rope().slice(range.0..range.1).to_string();
        if !current.starts_with('[') {
            return;
        }
        let new = markdown::toggled_marker(&current);
        let sel = self
            .workbench
            .view(view)
            .map(|v| v.selection.clone())
            .unwrap_or_default();
        let changes = ChangeSet::replace(range.0, range.1, new);
        let sel = sel.map(&changes);
        let now = self.now_ms;
        let _ = self
            .workbench
            .edit(view, changes, sel, EditKind::Other, now);
    }

    /// Bold / italic / code: wraps the selection (or the word at the caret)
    /// with `marker`, or unwraps it when already wrapped.
    pub fn toggle_md_wrap(&mut self, view: ViewId, marker: &str) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(d) = self.workbench.document(v.document_id) else {
            return;
        };
        if d.is_read_only() {
            return;
        }
        let rope = d.buffer.rope();
        let p = v.selection.primary();
        let (a, b) = if p.is_empty() {
            notoir_editor::line_ops::word_at(rope, p.head)
        } else {
            (p.from(), p.to())
        };
        let n = marker.chars().count();
        let len = rope.len_chars();
        let before = (a >= n).then(|| rope.slice(a - n..a).to_string());
        let after = (b + n <= len).then(|| rope.slice(b..b + n).to_string());
        let wrapped = before.as_deref() == Some(marker) && after.as_deref() == Some(marker);
        let (changes, sel) = if wrapped {
            let cs = ChangeSet::new(vec![
                notoir_editor::Edit::new(a - n, a, ""),
                notoir_editor::Edit::new(b, b + n, ""),
            ]);
            (cs, Selection::new(vec![Range::new(a - n, b - n)], 0))
        } else if a == b {
            (
                ChangeSet::new(vec![notoir_editor::Edit::new(
                    a,
                    a,
                    format!("{marker}{marker}"),
                )]),
                Selection::caret(a + n),
            )
        } else {
            let cs = ChangeSet::new(vec![
                notoir_editor::Edit::new(a, a, marker.to_owned()),
                notoir_editor::Edit::new(b, b, marker.to_owned()),
            ]);
            (cs, Selection::new(vec![Range::new(a + n, b + n)], 0))
        };
        if let Ok(cs) = changes {
            let now = self.now_ms;
            let _ = self.workbench.edit(view, cs, sel, EditKind::Other, now);
        }
    }

    /// Insert Link: `[selection](url)` with the caret in the URL part.
    pub fn insert_md_link(&mut self, view: ViewId) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(d) = self.workbench.document(v.document_id) else {
            return;
        };
        let p = v.selection.primary();
        let text = d.buffer.rope().slice(p.from()..p.to()).to_string();
        let inserted = format!("[{text}]()");
        let caret = p.from() + inserted.chars().count() - 1;
        let changes = ChangeSet::replace(p.from(), p.to(), inserted.as_str());
        let now = self.now_ms;
        let _ = self
            .workbench
            .edit(view, changes, Selection::caret(caret), EditKind::Other, now);
    }

    pub(crate) fn place_caret_at(&mut self, view: ViewId, at: &OpenAt) {
        // Same document: reuse the open-file caret placement.
        let Some(path) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
            .and_then(|d| d.path().map(Path::to_path_buf))
        else {
            // Untitled / Scratchpad: anchors still work through the parse.
            let anchor = at.anchor.clone().unwrap_or_default();
            if let Some(doc) = self.workbench.view(view).map(|v| v.document_id)
                && let Some(d) = self.workbench.document(doc)
            {
                let md = markdown::parse(&d.buffer.text());
                if let Some(h) = markdown::find_heading(&md, &anchor) {
                    let line = d.buffer.rope().char_to_line(h.range.0);
                    let _ = self
                        .workbench
                        .set_selection(view, Selection::caret(h.range.0));
                    self.find.scroll.insert(
                        view,
                        notoir_core::view::ScrollPosition {
                            top_line: line,
                            ..Default::default()
                        },
                    );
                }
            }
            return;
        };
        self.open_path(&path, at.clone());
    }

    /// Export as HTML…: asks where, next to the document by default.
    pub fn markdown_export(&mut self, doc: DocumentId) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let stem = match &d.origin {
            notoir_core::document::DocumentOrigin::File { path, .. } => path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => self.document_title(doc),
        };
        let dir = d
            .path()
            .and_then(|p| p.parent())
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.workspace.as_ref().map(|w| w.root.clone()));
        let opts = notoir_platform::dialogs::FileDialogOptions {
            title: Some(crate::t!("dialogs:export.title")),
            directory: dir,
            file_name: Some(format!("{stem}.html")),
        };
        self.files.dialogs.export_file(opts, doc, self.io.poster());
    }

    /// Renders and writes the page on the IO thread (through the save
    /// writer: atomic, UTF-8).
    pub fn markdown_export_to(&mut self, doc: DocumentId, path: Option<std::path::PathBuf>) {
        let (Some(path), Some(d)) = (path, self.workbench.document(doc)) else {
            return;
        };
        let text = d.buffer.text();
        let fallback = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lang = crate::i18n::language().code().to_owned();
        self.io.submit(move || {
            let page = notoir_editor::markdown::to_html(&text);
            let title = page.title.unwrap_or(fallback);
            let html = notoir_editor::markdown::html_document(&title, &page.body, &lang);
            let req = notoir_workspace::fs_io::SaveRequest {
                path: path.clone(),
                text: html,
                encoding: notoir_editor::encoding::TextEncoding::UTF8,
                line_ending: notoir_editor::LineEnding::Lf,
                expected_hash: None,
                // The dialog already confirmed replacing an existing file.
                overwrite: true,
                atomic: true,
            };
            let result = notoir_workspace::fs_io::save(&req)
                .map(|_| ())
                .map_err(|e| e.to_string());
            Some(IoResult::Exported { path, result })
        });
    }

    pub fn markdown_exported(&mut self, path: std::path::PathBuf, result: Result<(), String>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match result {
            Ok(()) => self.notify(Notification::new(
                Severity::Info,
                Translatable::new("notifications:exported").arg("name", name),
            )),
            Err(e) => self.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("notifications:exportFailed").arg("name", name),
                )
                .detail(e),
            ),
        }
    }
}
