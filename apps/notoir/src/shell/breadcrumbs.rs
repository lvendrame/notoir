//! Breadcrumbs (docs/specs/26 §Breadcrumbs): workspace-relative folders, the
//! file, then the symbol path at the cursor. Folder and file segments drop
//! down their directory's entries; symbol segments their sibling symbols.

use std::path::{Path, PathBuf};

use egui::{Rect, RichText, Sense, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::explorer::{Children, NodeKind};
use notoir_core::ids::ViewId;
use notoir_editor::outline::{Symbol, path_at};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::t;
use crate::theme::Tok;

/// Bar height (spec 21).
pub const HEIGHT: f32 = 22.0;

enum Segment {
    /// A folder (dropdown: its entries).
    Dir { name: String, path: PathBuf },
    /// The file (dropdown: its folder's entries).
    File { name: String, dir: Option<PathBuf> },
    /// Collapsed leading path outside the workspace.
    Ellipsis,
    /// A symbol (dropdown: its siblings).
    Symbol {
        name: String,
        siblings: Vec<(String, usize)>,
    },
}

impl Segment {
    fn label(&self) -> &str {
        match self {
            Segment::Dir { name, .. }
            | Segment::File { name, .. }
            | Segment::Symbol { name, .. } => name,
            Segment::Ellipsis => "…",
        }
    }
}

fn segments(core: &mut AppCore, view: ViewId) -> Vec<Segment> {
    let mut out = Vec::new();
    let Some(v) = core.workbench.view(view) else {
        return out;
    };
    let doc_id = v.document_id;
    let caret = v.selection.primary().head;
    let Some(doc) = core.workbench.document(doc_id) else {
        return out;
    };
    match doc.path() {
        Some(path) => {
            let path = path.to_path_buf();
            let root = core.workspace.as_ref().map(|w| w.root.clone());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match root.as_ref().filter(|r| path.starts_with(r)) {
                Some(root) => {
                    let mut dir = root.clone();
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    let parts: Vec<_> = rel.components().collect();
                    for c in &parts[..parts.len().saturating_sub(1)] {
                        dir.push(c);
                        out.push(Segment::Dir {
                            name: c.as_os_str().to_string_lossy().into_owned(),
                            path: dir.clone(),
                        });
                    }
                }
                None => {
                    out.push(Segment::Ellipsis);
                    if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
                        out.push(Segment::Dir {
                            name: parent.to_string_lossy().into_owned(),
                            path: path.parent().map(Path::to_path_buf).unwrap_or_default(),
                        });
                    }
                }
            }
            out.push(Segment::File {
                name,
                dir: path.parent().map(Path::to_path_buf),
            });
        }
        None => {
            let name = core.document_title(doc_id);
            out.push(Segment::File { name, dir: None });
        }
    }
    if let Some(s) = core.structure(doc_id) {
        let chain = path_at(&s.symbols, caret);
        let mut level: &[Symbol] = &s.symbols;
        for sym in chain {
            out.push(Segment::Symbol {
                name: sym.name.clone(),
                siblings: level
                    .iter()
                    .map(|x| (x.name.clone(), x.selection_range.0))
                    .collect(),
            });
            level = &sym.children;
        }
    }
    out
}

/// Entries of `dir` from the workspace tree; requests a listing when needed.
fn entries(core: &mut AppCore, dir: &Path) -> Option<Vec<(String, PathBuf, bool)>> {
    let ws = core.workspace.as_ref()?;
    let node = ws.tree.node(dir);
    match node.map(|n| &n.children) {
        Some(Children::Loaded(kids)) => {
            let mut list: Vec<(String, PathBuf, bool)> = kids
                .iter()
                .filter_map(|k| ws.tree.node(k))
                .map(|n| {
                    (
                        n.info.name.clone(),
                        n.path.clone(),
                        n.info.kind == NodeKind::Dir,
                    )
                })
                .collect();
            list.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });
            Some(list)
        }
        Some(Children::Loading) => None,
        _ => {
            core.request_listing(dir);
            None
        }
    }
}

fn dir_menu(ui: &mut egui::Ui, core: &mut AppCore, dir: &Path, actions: &mut Actions) {
    match entries(core, dir) {
        None => {
            ui.label(RichText::new(t!("panels:breadcrumbs.loading")).size(12.0));
        }
        Some(list) if list.is_empty() => {
            ui.label(RichText::new(t!("panels:breadcrumbs.empty")).size(12.0));
        }
        Some(list) => {
            for (name, path, is_dir) in list {
                if is_dir {
                    ui.menu_button(format!("{name}/"), |ui| dir_menu(ui, core, &path, actions));
                } else if ui.button(&name).clicked() {
                    actions.push(
                        CommandInvocation::new("explorer.open", InvocationSource::Internal)
                            .with_args(json!({ "path": path.to_string_lossy() })),
                    );
                    ui.close();
                }
            }
        }
    }
}

/// Draws the bar for `view` in `rect`.
pub fn bar(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    rect: Rect,
    view: ViewId,
    actions: &mut Actions,
) {
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let segs = segments(core, view);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(rect);
    child.spacing_mut().item_spacing.x = 2.0;
    let open_last = std::mem::take(&mut core.breadcrumbs_focus);
    let n = segs.len();
    for (i, seg) in segs.iter().enumerate() {
        if i > 0 {
            child.label(
                RichText::new("›")
                    .size(12.0)
                    .color(kit.color(Tok::TextTertiary)),
            );
        }
        let last = i + 1 == n;
        let color = kit.color(if last {
            Tok::TextPrimary
        } else {
            Tok::TextSecondary
        });
        let galley = child.painter().layout_no_wrap(
            seg.label().to_owned(),
            egui::FontId::proportional(12.0),
            color,
        );
        let (r, resp) =
            child.allocate_exact_size(galley.size() + Vec2::new(6.0, 4.0), Sense::click());
        if resp.hovered() {
            child
                .painter()
                .rect_filled(r, 3.0, kit.color(Tok::BgPanelRaised));
        }
        child
            .painter()
            .galley(r.center() - galley.size() / 2.0, galley, color);
        // Its own accessible name: the file segment would clash with the tab.
        let a11y = t!("panels:breadcrumbs.segment", name = seg.label());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a11y));
        if open_last && last {
            egui::Popup::open_id(child.ctx(), egui::Popup::default_response_id(&resp));
        }
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(180.0);
            ui.set_max_height(360.0);
            egui::ScrollArea::vertical().show(ui, |ui| match seg {
                Segment::Dir { path, .. } => dir_menu(ui, core, path, actions),
                Segment::File { dir: Some(dir), .. } => dir_menu(ui, core, dir, actions),
                Segment::File { dir: None, .. } | Segment::Ellipsis => {
                    ui.label(RichText::new(t!("panels:breadcrumbs.empty")).size(12.0));
                }
                Segment::Symbol { siblings, name } => {
                    for (s, pos) in siblings {
                        if ui.add(egui::Button::selectable(s == name, s)).clicked() {
                            actions.push(
                                CommandInvocation::new(
                                    "navigate.revealRange",
                                    InvocationSource::Internal,
                                )
                                .with_args(json!({ "start": pos, "end": pos, "view": view.0 })),
                            );
                            ui.close();
                        }
                    }
                }
            });
        });
    }
}
