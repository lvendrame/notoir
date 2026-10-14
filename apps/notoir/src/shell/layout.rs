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
    handle_file_drop(ui, &areas, core);
    out
}

struct Renderer<'a, 'k> {
    kit: &'a mut Kit<'k>,
    core: &'a mut AppCore,
    editors: &'a mut EditorCaches,
    drag: &'a mut Option<TabDrag>,
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
        let strip = tabs::strip(
            &mut child,
            self.kit,
            self.core,
            group,
            self.context,
            self.actions,
            self.drag,
            &mut tab_rects,
        );
        let body = Rect::from_min_max(Pos2::new(rect.left(), strip.bottom()), rect.max);
        let active = self
            .core
            .workbench
            .layout
            .group(group)
            .and_then(|g| g.active);
        match active {
            Some(view) => {
                let opts = self.core.editor_options(view);
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
                if out.interacted && self.core.workbench.active_view() != Some(view) {
                    self.out.activate = Some(view);
                }
            }
            None => {
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
        self.areas.push(GroupArea {
            group,
            strip,
            body,
            tabs: tab_rects,
        });
    }
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

/// Files dropped from the OS open in the group under the pointer.
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
        core.open_path(&path, crate::files::OpenAt::default());
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
