//! Editor area: the split layout tree with splitters, one tab strip and
//! editor per group, tab drag-and-drop with docking targets, and OS file drops
//! (docs/specs/03).

use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::{GroupId, ViewId};
use notoir_core::layout::{Axis, Direction, LayoutNode};
use notoir_core::when::ContextSnapshot;
use serde_json::json;

use super::{Actions, Kit, empty_state, tabs};
use crate::core::AppCore;
use crate::editor_widget::{self, EditorCaches};
use crate::fonts::CjkUse;
use crate::menus;
use crate::t;
use crate::theme::Tok;

/// Splitter hit area thickness (the visible line is 1 px).
const SPLITTER: f32 = 5.0;
/// Minimum group size along a split axis (spec 03).
const MIN_GROUP: f32 = 120.0;
/// Edge zone of a group body that docks into a new split.
const EDGE_ZONE: f32 = 0.25;

/// A tab being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabDrag {
    pub view: ViewId,
    pub from: GroupId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// Insert into a tab strip at a position.
    Strip { group: GroupId, index: usize },
    /// Add to a group as a tab.
    Center(GroupId),
    /// Dock into a new split next to a group.
    Edge { group: GroupId, dir: Direction },
}

struct GroupArea {
    group: GroupId,
    strip: Rect,
    body: Rect,
    tabs: Vec<(ViewId, Rect)>,
}

#[derive(Default)]
pub struct LayoutOutput {
    pub editor_focused: bool,
    pub zoom_steps: i32,
    pub cjk: CjkUse,
    /// The view the user clicked into (becomes active).
    pub activate: Option<ViewId>,
    /// Splitter drags: (split path, divider index, delta as fraction).
    pub resizes: Vec<(Vec<usize>, usize, f32, f32)>,
}

/// Renders the editor area into `ui`'s remaining space.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    editors: &mut EditorCaches,
    drag: &mut Option<TabDrag>,
    path_drag: &mut Option<std::path::PathBuf>,
    previews: &mut super::md_preview::Previews,
    trees: &mut super::json_tree::JsonTrees,
    focus_request: Option<ViewId>,
    context: &ContextSnapshot,
    actions: &mut Actions,
    now: f64,
) -> LayoutOutput {
    let rect = ui.available_rect_before_wrap();
    ui.advance_cursor_after_rect(rect);
    let mut out = LayoutOutput::default();
    let mut areas = Vec::new();
    let root = match core.workbench.layout.maximized {
        Some(g) => LayoutNode::Group(g),
        None => core.workbench.layout.root.clone(),
    };
    let only_group = core.workbench.layout.groups.len() == 1;
    let mut r = Renderer {
        kit,
        core,
        editors,
        drag,
        previews,
        trees,
        focus_request,
        context,
        actions,
        now,
        out: &mut out,
        areas: &mut areas,
        only_group,
    };
    r.node(ui, &root, rect, &mut Vec::new());
    handle_drag(ui, kit, drag, &areas, actions);
    handle_path_drag(ui, kit, path_drag, &areas, core);
    handle_file_drop(ui, &areas, core);
    out
}

struct Renderer<'a, 'k> {
    kit: &'a mut Kit<'k>,
    core: &'a mut AppCore,
    editors: &'a mut EditorCaches,
    drag: &'a mut Option<TabDrag>,
    previews: &'a mut super::md_preview::Previews,
    trees: &'a mut super::json_tree::JsonTrees,
    focus_request: Option<ViewId>,
    context: &'a ContextSnapshot,
    actions: &'a mut Actions,
    now: f64,
    out: &'a mut LayoutOutput,
    areas: &'a mut Vec<GroupArea>,
    only_group: bool,
}

impl Renderer<'_, '_> {
    fn node(&mut self, ui: &mut egui::Ui, node: &LayoutNode, rect: Rect, path: &mut Vec<usize>) {
        match node {
            LayoutNode::Group(g) => self.group(ui, *g, rect),
            LayoutNode::Split { axis, children } => {
                let horizontal = *axis == Axis::Horizontal;
                let total = if horizontal {
                    rect.width()
                } else {
                    rect.height()
                };
                let usable = total - SPLITTER * (children.len() - 1) as f32;
                let mut start = if horizontal { rect.left() } else { rect.top() };
                for (i, (child, frac)) in children.iter().enumerate() {
                    let len = usable * frac;
                    let child_rect = if horizontal {
                        Rect::from_min_size(
                            Pos2::new(start, rect.top()),
                            Vec2::new(len, rect.height()),
                        )
                    } else {
                        Rect::from_min_size(
                            Pos2::new(rect.left(), start),
                            Vec2::new(rect.width(), len),
                        )
                    };
                    path.push(i);
                    self.node(ui, child, child_rect, path);
                    path.pop();
                    start += len;
                    if i + 1 < children.len() {
                        let split = if horizontal {
                            Rect::from_min_size(
                                Pos2::new(start, rect.top()),
                                Vec2::new(SPLITTER, rect.height()),
                            )
                        } else {
                            Rect::from_min_size(
                                Pos2::new(rect.left(), start),
                                Vec2::new(rect.width(), SPLITTER),
                            )
                        };
                        self.splitter(ui, split, horizontal, path, i, usable);
                        start += SPLITTER;
                    }
                }
            }
        }
    }

    fn splitter(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        horizontal: bool,
        path: &[usize],
        index: usize,
        usable: f32,
    ) {
        let id = egui::Id::new(("splitter", path.to_vec(), index));
        let resp = ui.interact(rect, id, Sense::drag());
        let active = resp.hovered() || resp.dragged();
        if active {
            ui.ctx().set_cursor_icon(if horizontal {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            });
        }
        let color = if active {
            self.kit.color(Tok::Accent)
        } else {
            self.kit.color(Tok::Border)
        };
        let c = rect.center();
        let line = if horizontal {
            [Pos2::new(c.x, rect.top()), Pos2::new(c.x, rect.bottom())]
        } else {
            [Pos2::new(rect.left(), c.y), Pos2::new(rect.right(), c.y)]
        };
        ui.painter().line_segment(line, Stroke::new(1.0, color));
        if resp.dragged() && usable > 0.0 {
            let d = resp.drag_delta();
            let delta = if horizontal { d.x } else { d.y } / usable;
            if delta != 0.0 {
                self.out
                    .resizes
                    .push((path.to_vec(), index, delta, MIN_GROUP / usable));
            }
        }
    }

    fn group(&mut self, ui: &mut egui::Ui, group: GroupId, rect: Rect) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(rect);
        let has_tabs = self
            .core
            .workbench
            .layout
            .group(group)
            .is_some_and(|g| !g.tabs.is_empty());
        if !has_tabs && self.only_group {
            empty_state::show(&mut child, self.kit, self.core, self.actions);
            self.areas.push(GroupArea {
                group,
                strip: Rect::NOTHING,
                body: rect,
                tabs: Vec::new(),
            });
            return;
        }
        let mut tab_rects = Vec::new();
        let hide_tabs =
            self.core.zen.is_some() && self.core.settings.bool("zenMode.hideTabs", None);
        let strip = if hide_tabs {
            Rect::from_min_size(rect.min, Vec2::new(rect.width(), 0.0))
        } else {
            tabs::strip(
                &mut child,
                self.kit,
                self.core,
                group,
                self.context,
                self.actions,
                self.drag,
                &mut tab_rects,
            )
        };
        let body = Rect::from_min_max(Pos2::new(rect.left(), strip.bottom()), rect.max);
        let body_full = body;
        let active = self
            .core
            .workbench
            .layout
            .group(group)
            .and_then(|g| g.active);
        let kind = active
            .and_then(|v| self.core.workbench.view(v))
            .map(|v| v.kind);
        match (active, kind) {
            (Some(view), Some(notoir_core::view::ViewKind::Settings)) => {
                let mut body_ui = child.new_child(egui::UiBuilder::new().max_rect(body));
                body_ui.set_clip_rect(body);
                super::settings_ui::show(&mut body_ui, self.kit, self.core, self.actions);
                if body_ui.ui_contains_pointer()
                    && body_ui.input(|i| i.pointer.any_pressed())
                    && self.core.workbench.active_view() != Some(view)
                {
                    self.out.activate = Some(view);
                }
            }
            (Some(view), Some(notoir_core::view::ViewKind::Welcome)) => {
                let mut body_ui = child.new_child(egui::UiBuilder::new().max_rect(body));
                body_ui.set_clip_rect(body);
                super::welcome::show(&mut body_ui, self.kit, self.core, self.actions);
                let _ = view;
            }
            (Some(view), _) => {
                if let Some(d) = self.core.workbench.view(view).map(|v| v.document_id) {
                    self.core.structure(d);
                }
                let opts = self.core.editor_options(view);
                let doc = self.core.workbench.view(view).map(|v| v.document_id);
                // Breadcrumbs under the tab strip (spec 26).
                let body = if self
                    .core
                    .settings
                    .bool("workbench.breadcrumbs.enabled", None)
                    && self.core.zen.is_none()
                {
                    let bar = Rect::from_min_size(
                        body.min,
                        Vec2::new(body.width(), super::breadcrumbs::HEIGHT),
                    );
                    super::breadcrumbs::bar(
                        &mut child,
                        self.kit,
                        self.core,
                        bar,
                        view,
                        self.actions,
                    );
                    Rect::from_min_max(Pos2::new(body.left(), bar.bottom()), body.max)
                } else {
                    body
                };
                let banner = doc.and_then(|d| self.core.banners.get(&d).copied().map(|b| (d, b)));
                let body = match banner {
                    Some((doc, b)) => {
                        let bar = Rect::from_min_size(body.min, Vec2::new(body.width(), BANNER_H));
                        banner_bar(
                            &mut child,
                            self.kit,
                            self.core,
                            bar,
                            doc,
                            view,
                            b,
                            self.actions,
                        );
                        Rect::from_min_max(Pos2::new(body.left(), bar.bottom()), body.max)
                    }
                    None => body,
                };
                // Markdown modes: editor, preview, or both (spec 24).
                use notoir_core::view::ViewKind as VK;
                let view_kind = kind.unwrap_or(VK::Text);
                let is_md = doc.is_some_and(|d| self.core.is_markdown(d));
                let is_json = doc.is_some_and(|d| self.core.is_json(d));
                let mode = match view_kind {
                    VK::MarkdownPreview | VK::MarkdownSplit if is_md => view_kind,
                    VK::JsonTree | VK::JsonSplit if is_json => view_kind,
                    _ => VK::Text,
                };
                let (editor_rect, preview_rect) = match mode {
                    VK::MarkdownPreview | VK::JsonTree => (None, Some(body)),
                    VK::MarkdownSplit | VK::JsonSplit => {
                        let ratio = self
                            .core
                            .workbench
                            .view(view)
                            .map_or(0.5, |v| v.preview_ratio)
                            .clamp(0.2, 0.8);
                        let orientation = self
                            .core
                            .settings
                            .str("markdown.splitOrientation", None)
                            .to_owned();
                        let vertical = orientation == "vertical"
                            || (orientation == "auto" && body.width() < 700.0);
                        let (a, split, b) = if vertical {
                            let y = body.top() + body.height() * ratio;
                            (
                                Rect::from_min_max(body.min, Pos2::new(body.right(), y - 2.0)),
                                Rect::from_min_max(
                                    Pos2::new(body.left(), y - 2.0),
                                    Pos2::new(body.right(), y + 3.0),
                                ),
                                Rect::from_min_max(Pos2::new(body.left(), y + 3.0), body.max),
                            )
                        } else {
                            let x = body.left() + body.width() * ratio;
                            (
                                Rect::from_min_max(body.min, Pos2::new(x - 2.0, body.bottom())),
                                Rect::from_min_max(
                                    Pos2::new(x - 2.0, body.top()),
                                    Pos2::new(x + 3.0, body.bottom()),
                                ),
                                Rect::from_min_max(Pos2::new(x + 3.0, body.top()), body.max),
                            )
                        };
                        let resp =
                            child.interact(split, egui::Id::new(("md-split", view)), Sense::drag());
                        let active = resp.hovered() || resp.dragged();
                        if active {
                            child.ctx().set_cursor_icon(if vertical {
                                egui::CursorIcon::ResizeVertical
                            } else {
                                egui::CursorIcon::ResizeHorizontal
                            });
                        }
                        let c = split.center();
                        let line = if vertical {
                            [Pos2::new(split.left(), c.y), Pos2::new(split.right(), c.y)]
                        } else {
                            [Pos2::new(c.x, split.top()), Pos2::new(c.x, split.bottom())]
                        };
                        child.painter().line_segment(
                            line,
                            Stroke::new(
                                1.0,
                                if active {
                                    self.kit.color(Tok::Accent)
                                } else {
                                    self.kit.color(Tok::Border)
                                },
                            ),
                        );
                        if resp.dragged() {
                            let d = resp.drag_delta();
                            let delta = if vertical {
                                d.y / body.height()
                            } else {
                                d.x / body.width()
                            };
                            if let Some(v) = self.core.workbench.views.get_mut(view) {
                                v.preview_ratio = (v.preview_ratio + delta).clamp(0.2, 0.8);
                            }
                        }
                        (Some(a), Some(b))
                    }
                    _ => (Some(body), None),
                };
                if let (Some(rect), Some(doc), true) = (preview_rect, doc, is_json) {
                    if let Some((roots, error, pending)) = self.core.json_doc(doc) {
                        let mut tui = child.new_child(egui::UiBuilder::new().max_rect(rect));
                        tui.set_clip_rect(rect);
                        super::json_tree::show(
                            &mut tui,
                            self.kit,
                            self.core,
                            view,
                            doc,
                            &roots,
                            error.as_ref(),
                            self.trees,
                            self.actions,
                        );
                        if pending {
                            child
                                .ctx()
                                .request_repaint_after(std::time::Duration::from_millis(
                                    crate::json::PARSE_DEBOUNCE_MS,
                                ));
                        }
                        if tui.ui_contains_pointer()
                            && tui.input(|i| i.pointer.any_pressed())
                            && self.core.workbench.active_view() != Some(view)
                        {
                            self.out.activate = Some(view);
                        }
                    }
                } else if let (Some(rect), Some(doc)) = (preview_rect, doc)
                    && let Some((md, pending)) = self.core.markdown_doc(doc)
                {
                    {
                        let mut pui = child.new_child(egui::UiBuilder::new().max_rect(rect));
                        pui.set_clip_rect(rect);
                        super::md_preview::show(
                            &mut pui,
                            self.kit,
                            self.core,
                            view,
                            doc,
                            &md,
                            self.previews,
                            self.actions,
                        );
                        if pending {
                            child
                                .ctx()
                                .request_repaint_after(std::time::Duration::from_millis(
                                    crate::markdown::PARSE_DEBOUNCE_MS,
                                ));
                        }
                        if pui.ui_contains_pointer()
                            && pui.input(|i| i.pointer.any_pressed())
                            && self.core.workbench.active_view() != Some(view)
                        {
                            self.out.activate = Some(view);
                        }
                    }
                }
                let Some(body) = editor_rect else {
                    return self.finish_group(group, strip, body_full, tab_rects);
                };
                let mut body_ui = child.new_child(egui::UiBuilder::new().max_rect(body));
                let out = editor_widget::show(
                    &mut body_ui,
                    self.kit,
                    &mut self.core.workbench,
                    self.editors,
                    view,
                    &opts,
                    self.focus_request == Some(view),
                    self.now,
                );
                self.out.editor_focused |= out.focused;
                self.out.zoom_steps += out.zoom_steps;
                self.out.cjk.merge(out.cjk);
                self.actions.0.extend(out.commands);
                if let Some(resp) = &out.response {
                    let gutter_line = out.gutter_menu_line;
                    resp.context_menu(|ui| {
                        ui.set_min_width(220.0);
                        let items = match gutter_line {
                            Some(line) => gutter_context_items(line),
                            None => editor_context_items(),
                        };
                        let items = menus::visible_items(&items, &self.core.commands);
                        super::chrome::menu_items(
                            ui,
                            self.kit,
                            self.core,
                            self.context,
                            &items,
                            self.actions,
                        );
                    });
                }
                super::find_widget::show(&mut child, self.kit, self.core, view, body, self.actions);
                if out.interacted && self.core.workbench.active_view() != Some(view) {
                    self.out.activate = Some(view);
                }
                if mode == VK::MarkdownSplit
                    && self.core.settings.bool("markdown.preview.scrollSync", None)
                {
                    self.sync_scroll(view);
                }
            }
            (None, _) => {
                child
                    .painter()
                    .rect_filled(body, 0.0, self.kit.color(Tok::BgEditor));
                child.painter().text(
                    body.center(),
                    egui::Align2::CENTER_CENTER,
                    t!("tabs:dropHere"),
                    egui::FontId::proportional(13.0),
                    self.kit.color(Tok::TextTertiary),
                );
            }
        }
        self.finish_group(group, strip, body_full, tab_rects);
    }

    fn finish_group(&mut self, group: GroupId, strip: Rect, body: Rect, tabs: Vec<(ViewId, Rect)>) {
        self.areas.push(GroupArea {
            group,
            strip,
            body,
            tabs,
        });
    }

    /// Markdown split: whichever side the user scrolled drives the other
    /// through the preview's block positions (spec 24 §Scroll Synchronization).
    fn sync_scroll(&mut self, view: ViewId) {
        let Some(editor_top) = self.editors.top_line(view) else {
            return;
        };
        let Some(st) = self.previews.views.get_mut(&view) else {
            return;
        };
        if st.blocks.is_empty() {
            return;
        }
        if st.last_editor_top != Some(editor_top) {
            // Editor moved: put the matching block (interpolated) at the top.
            let y = match st
                .blocks
                .iter()
                .find(|b| b.0 <= editor_top && editor_top <= b.1)
            {
                Some(&(l0, l1, y0, y1)) => {
                    let frac = (editor_top - l0) as f32 / ((l1 - l0 + 1) as f32);
                    y0 + frac * (y1 - y0)
                }
                None => st
                    .blocks
                    .iter()
                    .rev()
                    .find(|b| b.1 < editor_top)
                    .map_or(0.0, |b| b.3),
            };
            st.set_offset = Some(y);
            st.last_editor_top = Some(editor_top);
        } else if st
            .synced_offset
            .is_some_and(|o| (o - st.offset).abs() > 1.0)
        {
            // Preview scrolled by the user: move the editor.
            let y = st.offset;
            let line = match st.blocks.iter().find(|b| b.2 <= y && y < b.3) {
                Some(&(l0, l1, y0, y1)) => {
                    l0 + (((y - y0) / (y1 - y0).max(1.0)) * (l1 - l0 + 1) as f32) as usize
                }
                None => st.blocks.last().map_or(0, |b| b.1),
            };
            self.core.find.scroll.insert(
                view,
                notoir_core::view::ScrollPosition {
                    top_line: line,
                    ..Default::default()
                },
            );
            st.last_editor_top = Some(line);
            st.synced_offset = Some(st.offset);
        }
    }
}

const BANNER_H: f32 = 34.0;

/// The editor's right-click menu (spec 11 §Editor Context Menu); items whose
/// commands do not exist yet are left out.
/// Gutter context menu for `line` (spec 12 §Gutter).
fn gutter_context_items(line: usize) -> Vec<menus::MenuItem> {
    use menus::MenuItem;
    let at = |id: &'static str| MenuItem::Command {
        id,
        args: json!({ "line": line }),
        label: None,
        checked: None,
    };
    let toggle = |id: &'static str, checked: &str| MenuItem::Command {
        id,
        args: serde_json::Value::Null,
        label: None,
        checked: Some(checked.to_owned()),
    };
    vec![
        at("bookmarks.toggle"),
        at("editor.copyLineNumber"),
        MenuItem::Submenu {
            label: "menus:sub.lineNumbers",
            items: vec![
                toggle("view.toggleLineNumbers", "lineNumbersVisible"),
                toggle("view.toggleRelativeLineNumbers", "lineNumbersRelative"),
            ],
        },
        MenuItem::Separator,
        at("fold.toggle"),
    ]
}

fn editor_context_items() -> Vec<menus::MenuItem> {
    use menus::MenuItem;
    let cmd = |id: &'static str| MenuItem::Command {
        id,
        args: serde_json::Value::Null,
        label: None,
        checked: None,
    };
    vec![
        cmd("edit.cut"),
        cmd("edit.copy"),
        cmd("edit.paste"),
        cmd("edit.pastePlain"),
        MenuItem::Separator,
        cmd("edit.selectAll"),
        MenuItem::Separator,
        cmd("edit.find"),
        cmd("edit.replace"),
        MenuItem::Separator,
        cmd("navigate.goToLine"),
        cmd("bookmarks.toggle"),
        MenuItem::Separator,
        MenuItem::Submenu {
            label: "menus:sub.convertCase",
            items: vec![
                cmd("edit.transformToUppercase"),
                cmd("edit.transformToLowercase"),
                cmd("edit.transformToTitleCase"),
            ],
        },
        MenuItem::Submenu {
            label: "menus:sub.sortLines",
            items: vec![cmd("edit.sortLines"), cmd("edit.sortLinesDescending")],
        },
        MenuItem::Separator,
        cmd("editor.formatDocument"),
        cmd("compare.currentChanges"),
    ]
}

/// The bar above an editor whose file changed or vanished on disk, or whose
/// recovered text conflicts with the disk (docs/specs/09, 10).
#[allow(clippy::too_many_arguments)]
fn banner_bar(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    rect: Rect,
    doc: notoir_core::ids::DocumentId,
    view: ViewId,
    banner: crate::recovery::Banner,
    actions: &mut Actions,
) {
    use crate::recovery::BannerKind;
    let name = core.document_title(doc);
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgPanelRaised));
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, kit.color(Tok::Border)),
    );
    let (icon, message, buttons): (crate::icons::Icon, String, Vec<(&str, &'static str)>) =
        match banner.kind {
            BannerKind::Changed => (
                crate::icons::Icon::RefreshCw,
                t!("banners:changed", name = name),
                vec![
                    ("banners:reload", "file.reloadFromDisk"),
                    ("banners:keepMine", "file.keepMine"),
                ],
            ),
            BannerKind::Deleted => (
                crate::icons::Icon::FileX,
                t!("banners:deleted", name = name),
                vec![
                    ("banners:save", "file.save"),
                    ("banners:close", "file.close"),
                ],
            ),
            BannerKind::RecoveredConflict => (
                crate::icons::Icon::TriangleAlert,
                t!("banners:recoveredConflict"),
                vec![
                    ("banners:keepRecovered", "file.keepMine"),
                    ("banners:useDisk", "file.useDiskVersion"),
                ],
            ),
        };
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bar.add(kit.icon(
        bar.ctx(),
        icon,
        crate::theme::ICON_SM,
        kit.color(Tok::Warning),
    ));
    bar.label(
        egui::RichText::new(message)
            .size(13.0)
            .color(kit.color(Tok::TextPrimary)),
    );
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for (label, id) in buttons.into_iter().rev() {
            if ui.button(t!(label)).clicked() {
                actions.push(
                    CommandInvocation::new(id, InvocationSource::Internal)
                        .with_args(json!({ "doc": doc.0, "view": view.0 })),
                );
            }
        }
    });
}

/// Where a drop at `p` would land.
fn target_at(areas: &[GroupArea], p: Pos2) -> Option<DropTarget> {
    for a in areas {
        if a.strip.contains(p) {
            let index = a.tabs.iter().filter(|(_, r)| r.center().x < p.x).count();
            return Some(DropTarget::Strip {
                group: a.group,
                index,
            });
        }
        if a.body.contains(p) {
            let rel = (p - a.body.min) / a.body.size();
            let edges = [
                (rel.x, Direction::Left),
                (1.0 - rel.x, Direction::Right),
                (rel.y, Direction::Up),
                (1.0 - rel.y, Direction::Down),
            ];
            let (dist, dir) = edges
                .into_iter()
                .fold((f32::MAX, Direction::Left), |best, e| {
                    if e.0 < best.0 { e } else { best }
                });
            return Some(if dist < EDGE_ZONE {
                DropTarget::Edge {
                    group: a.group,
                    dir,
                }
            } else {
                DropTarget::Center(a.group)
            });
        }
    }
    None
}

/// Preview rectangle of a drop target.
fn preview_rect(areas: &[GroupArea], target: DropTarget) -> Option<Rect> {
    let area = |g: GroupId| areas.iter().find(|a| a.group == g);
    match target {
        DropTarget::Strip { group, index } => {
            let a = area(group)?;
            let x = match a.tabs.get(index) {
                Some((_, r)) => r.left(),
                None => a.tabs.last().map_or(a.strip.left(), |(_, r)| r.right()),
            };
            Some(Rect::from_min_max(
                Pos2::new(x - 1.0, a.strip.top() + 4.0),
                Pos2::new(x + 1.0, a.strip.bottom() - 4.0),
            ))
        }
        DropTarget::Center(g) => area(g).map(|a| a.body),
        DropTarget::Edge { group, dir } => {
            let b = area(group)?.body;
            Some(match dir {
                Direction::Left => Rect::from_min_max(b.min, Pos2::new(b.center().x, b.bottom())),
                Direction::Right => Rect::from_min_max(Pos2::new(b.center().x, b.top()), b.max),
                Direction::Up => Rect::from_min_max(b.min, Pos2::new(b.right(), b.center().y)),
                Direction::Down => Rect::from_min_max(Pos2::new(b.left(), b.center().y), b.max),
            })
        }
    }
}

/// Paints the docking preview while dragging and turns the release into a
/// move/dock command (Alt/Option copies). Esc cancels.
fn handle_drag(
    ui: &egui::Ui,
    kit: &mut Kit,
    drag: &mut Option<TabDrag>,
    areas: &[GroupArea],
    actions: &mut Actions,
) {
    let Some(d) = *drag else { return };
    let (pos, down, esc, copy) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.key_pressed(egui::Key::Escape), i.modifiers.alt || i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: false, modifiers, .. } if modifiers.alt))));
    if esc {
        *drag = None;
        return;
    }
    let target = pos.and_then(|p| target_at(areas, p));
    if down {
        if let Some(rect) = target.and_then(|t| preview_rect(areas, t)) {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dock-preview"),
            ));
            painter.rect_filled(rect, 2.0, kit.color(Tok::AccentSoft));
            painter.rect_stroke(
                rect,
                2.0,
                Stroke::new(1.0, kit.color(Tok::Accent)),
                egui::StrokeKind::Inside,
            );
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        return;
    }
    *drag = None;
    let Some(target) = target else { return };
    let (id, args) = match target {
        DropTarget::Strip { group, index } => (
            "view.moveTab",
            json!({ "view": d.view.0, "group": group.0, "index": index, "copy": copy }),
        ),
        DropTarget::Center(group) => {
            if group == d.from && !copy {
                return;
            }
            (
                "view.moveTab",
                json!({ "view": d.view.0, "group": group.0, "copy": copy }),
            )
        }
        DropTarget::Edge { group, dir } => {
            let dir = match dir {
                Direction::Left => "left",
                Direction::Right => "right",
                Direction::Up => "up",
                Direction::Down => "down",
            };
            (
                "view.dockTab",
                json!({ "view": d.view.0, "group": group.0, "dir": dir, "copy": copy }),
            )
        }
    };
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

/// An Explorer entry dragged onto the editor area: files open in the group
/// (or dock into a new split on an edge).
fn handle_path_drag(
    ui: &egui::Ui,
    kit: &mut Kit,
    drag: &mut Option<std::path::PathBuf>,
    areas: &[GroupArea],
    core: &mut AppCore,
) {
    let Some(path) = drag.clone() else { return };
    let (pos, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
    let target = pos.and_then(|p| target_at(areas, p));
    if down {
        if let Some(rect) = target.and_then(|t| preview_rect(areas, t)) {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dock-preview"),
            ));
            painter.rect_filled(rect, 2.0, kit.color(Tok::AccentSoft));
            painter.rect_stroke(
                rect,
                2.0,
                Stroke::new(1.0, kit.color(Tok::Accent)),
                egui::StrokeKind::Inside,
            );
        }
        return;
    }
    let Some(target) = target else { return };
    *drag = None;
    if path.is_dir() {
        return;
    }
    let group = match target {
        DropTarget::Strip { group, .. } | DropTarget::Center(group) => Some(group),
        DropTarget::Edge { group, dir } => core.workbench.new_group_split(group, dir),
    };
    if let Some(g) = group {
        core.workbench.layout.active_group = g;
    }
    core.open_path(&path, crate::files::OpenAt::default());
}

/// Files dropped from the OS open in the group under the pointer; a folder
/// opens as the workspace.
fn handle_file_drop(ui: &egui::Ui, areas: &[GroupArea], core: &mut AppCore) {
    let (files, pos) = ui.input(|i| (i.raw.dropped_files.clone(), i.pointer.latest_pos()));
    if files.is_empty() {
        return;
    }
    if let Some(a) = pos.and_then(|p| {
        areas
            .iter()
            .find(|a| a.strip.contains(p) || a.body.contains(p))
    }) {
        core.workbench.layout.active_group = a.group;
    }
    for f in files {
        let path = f.path().to_path_buf();
        if path.is_dir() {
            core.open_workspace(&path);
        } else {
            core.open_path(&path, crate::files::OpenAt::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(group: u64, body: Rect) -> GroupArea {
        GroupArea {
            group: GroupId(group),
            strip: Rect::from_min_size(
                body.min - Vec2::new(0.0, 30.0),
                Vec2::new(body.width(), 30.0),
            ),
            body,
            tabs: Vec::new(),
        }
    }

    #[test]
    fn drop_zones() {
        let areas = [area(
            1,
            Rect::from_min_size(Pos2::new(0.0, 30.0), Vec2::new(400.0, 400.0)),
        )];
        assert_eq!(
            target_at(&areas, Pos2::new(200.0, 230.0)),
            Some(DropTarget::Center(GroupId(1)))
        );
        assert_eq!(
            target_at(&areas, Pos2::new(10.0, 230.0)),
            Some(DropTarget::Edge {
                group: GroupId(1),
                dir: Direction::Left
            })
        );
        assert_eq!(
            target_at(&areas, Pos2::new(200.0, 420.0)),
            Some(DropTarget::Edge {
                group: GroupId(1),
                dir: Direction::Down
            })
        );
        assert_eq!(
            target_at(&areas, Pos2::new(200.0, 10.0)),
            Some(DropTarget::Strip {
                group: GroupId(1),
                index: 0
            })
        );
        assert_eq!(target_at(&areas, Pos2::new(900.0, 900.0)), None);
    }
}
