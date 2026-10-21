//! Diff tab (docs/specs/27 §Diff View): a toolbar, the two side titles, two
//! editor panes aligned with filler rows and scrolled together, hunk copy
//! actions between them and an overview ruler.

use std::sync::Arc;

use egui::{Align2, Pos2, Rect, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::ViewId;
use notoir_core::when::ContextSnapshot;
use notoir_editor::diff::Side;
use serde_json::json;

use super::{Actions, Kit, icon_button, icon_button_sized};
use crate::compare::DiffSession;
use crate::core::AppCore;
use crate::editor_widget::{self, DiffDecor, EditorCaches, WrapMode};
use crate::fonts::CjkUse;
use crate::icons::Icon;
use crate::menus;
use crate::t;
use crate::theme::Tok;

const TOOLBAR_H: f32 = 32.0;
const TITLE_H: f32 = 24.0;
const INFO_H: f32 = 26.0;
/// Hunk actions between the panes.
const MIDDLE_W: f32 = 26.0;
const RULER_W: f32 = 12.0;

#[derive(Default)]
pub struct DiffOutput {
    pub focused: bool,
    /// Clicked into a pane: the tab becomes active.
    pub interacted: bool,
    pub zoom_steps: i32,
    pub cjk: CjkUse,
}

fn invoke(id: &'static str, args: serde_json::Value) -> CommandInvocation {
    CommandInvocation::new(id, InvocationSource::Internal).with_args(args)
}

/// Pane colouring for the session's current diff (rebuilt when it changes).
fn decor(s: &mut DiffSession) -> (DiffDecor, DiffDecor) {
    if let Some((d, l, r)) = &s.decor
        && Arc::ptr_eq(d, &s.diff)
    {
        return (l.clone(), r.clone());
    }
    let side = |side: Side| DiffDecor {
        width_cols: 0,
        max_lines: 0,
        inserted: side == Side::Right,
        lines: Arc::new(s.diff.changed_lines(side)),
        spans: Arc::new(s.diff.inline_spans(side)),
        fillers: Arc::new(s.diff.fillers(side)),
    };
    let (l, r) = (side(Side::Left), side(Side::Right));
    s.decor = Some((s.diff.clone(), l.clone(), r.clone()));
    (l, r)
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    editors: &mut EditorCaches,
    tab: ViewId,
    focus_request: Option<ViewId>,
    context: &ContextSnapshot,
    actions: &mut Actions,
    now: f64,
) -> DiffOutput {
    let mut out = DiffOutput::default();
    let rect = ui.available_rect_before_wrap();
    ui.advance_cursor_after_rect(rect);
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let Some(mut s) = core.compare.sessions.remove(&tab) else {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            t!("compare:unavailable"),
            egui::FontId::proportional(13.0),
            kit.color(Tok::TextTertiary),
        );
        return out;
    };
    let args = json!({ "view": tab.0 });

    // Toolbar: summary, Previous / Next Change, Ignore Whitespace, Swap.
    let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOOLBAR_H));
    ui.painter().rect_filled(bar, 0.0, kit.color(Tok::BgPanel));
    ui.painter().line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, kit.color(Tok::Border)),
    );
    let n = s.diff.hunks.len();
    let summary = if s.too_large {
        t!("compare:tooLarge")
    } else if s.is_computing() && n == 0 {
        t!("compare:computing")
    } else if n == 0 {
        t!("compare:identical")
    } else {
        crate::tn!("compare:changes", count = n)
    };
    let mut bui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(Vec2::new(10.0, 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bui.label(
        egui::RichText::new(summary)
            .size(12.5)
            .color(kit.color(Tok::TextSecondary)),
    );
    bui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if icon_button(
            ui,
            kit,
            Icon::ArrowLeftRight,
            &t!("commands:compare.swapSides"),
            false,
            true,
        )
        .clicked()
        {
            actions.push(invoke("compare.swapSides", args.clone()));
        }
        if icon_button(
            ui,
            kit,
            Icon::Space,
            &t!("commands:compare.toggleIgnoreWhitespace"),
            s.opts.ignore_whitespace,
            true,
        )
        .clicked()
        {
            actions.push(invoke("compare.toggleIgnoreWhitespace", args.clone()));
        }
        if icon_button(
            ui,
            kit,
            Icon::ArrowDown,
            &t!("commands:compare.nextChange"),
            false,
            n > 0,
        )
        .clicked()
        {
            actions.push(invoke("compare.nextChange", args.clone()));
        }
        if icon_button(
            ui,
            kit,
            Icon::ArrowUp,
            &t!("commands:compare.previousChange"),
            false,
            n > 0,
        )
        .clicked()
        {
            actions.push(invoke("compare.previousChange", args.clone()));
        }
    });

    let mut top = bar.bottom();
    // "Files are identical" info bar.
    if n == 0 && !s.is_computing() && !s.too_large {
        let info =
            Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(rect.width(), INFO_H));
        ui.painter()
            .rect_filled(info, 0.0, kit.color(Tok::BgPanelRaised));
        let icon = kit.icon(
            ui.ctx(),
            Icon::Info,
            crate::theme::ICON_SM,
            kit.color(Tok::Info),
        );
        icon.paint_at(
            ui,
            Rect::from_center_size(
                Pos2::new(info.left() + 18.0, info.center().y),
                Vec2::splat(crate::theme::ICON_SM),
            ),
        );
        let resp = ui.allocate_rect(info, Sense::hover());
        let text = t!("compare:filesIdentical");
        ui.painter().text(
            Pos2::new(info.left() + 32.0, info.center().y),
            Align2::LEFT_CENTER,
            &text,
            egui::FontId::proportional(12.5),
            kit.color(Tok::TextPrimary),
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
        top = info.bottom();
    }

    // Geometry: left pane | hunk actions | right pane | overview ruler.
    let body = Rect::from_min_max(Pos2::new(rect.left(), top + TITLE_H), rect.max);
    let pane_w = ((body.width() - MIDDLE_W - RULER_W) / 2.0).max(40.0);
    let left_rect = Rect::from_min_size(body.min, Vec2::new(pane_w, body.height()));
    let middle = Rect::from_min_size(
        Pos2::new(left_rect.right(), body.top()),
        Vec2::new(MIDDLE_W, body.height()),
    );
    let right_rect = Rect::from_min_size(
        Pos2::new(middle.right(), body.top()),
        Vec2::new(pane_w, body.height()),
    );
    let ruler = Rect::from_min_max(Pos2::new(right_rect.right(), body.top()), body.max);

    // Side titles (lock icon on read-only sides).
    for (side, r) in [(Side::Left, left_rect), (Side::Right, right_rect)] {
        let title = Rect::from_min_size(Pos2::new(r.left(), top), Vec2::new(r.width(), TITLE_H));
        ui.painter()
            .rect_filled(title, 0.0, kit.color(Tok::BgPanel));
        let pane = s.pane(side);
        let read_only = core
            .workbench
            .document(pane.doc)
            .is_some_and(|d| d.is_read_only());
        let mut x = title.left() + 10.0;
        if read_only {
            let icon = kit.icon(
                ui.ctx(),
                Icon::Lock,
                crate::theme::ICON_SM,
                kit.color(Tok::TextTertiary),
            );
            icon.paint_at(
                ui,
                Rect::from_center_size(
                    Pos2::new(x + 6.0, title.center().y),
                    Vec2::splat(crate::theme::ICON_SM),
                ),
            );
            x += 18.0;
        }
        let name = core.document_title(pane.doc);
        let dirty = core
            .workbench
            .document(pane.doc)
            .is_some_and(|d| d.is_dirty());
        let resp = ui.allocate_rect(title, Sense::hover());
        let text_rect = ui.painter().text(
            Pos2::new(x, title.center().y),
            Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(12.5),
            kit.color(Tok::TextSecondary),
        );
        // Unsaved: a dot after the name (as on tabs).
        if dirty {
            ui.painter().circle_filled(
                Pos2::new(text_rect.right() + 8.0, title.center().y),
                3.0,
                kit.color(Tok::TextSecondary),
            );
        }
        let label = if dirty {
            format!("{name} \u{25cf}")
        } else {
            name
        };
        let a11y = if read_only {
            format!("{label} ({})", t!("compare:readOnly"))
        } else {
            label
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &a11y));
    }
    ui.painter().line_segment(
        [
            Pos2::new(rect.left(), top + TITLE_H),
            Pos2::new(rect.right(), top + TITLE_H),
        ],
        Stroke::new(1.0, kit.color(Tok::Border)),
    );

    // Panes, scrolled together.
    let (mut ldec, mut rdec) = decor(&mut s);
    let mut width_cols = 0;
    let mut max_lines = 0;
    for pane in [&s.left, &s.right] {
        let opts = core.editor_options(pane.view);
        if let Some(doc) = core.workbench.document(pane.doc) {
            width_cols = width_cols.max(editors.width_cols(pane.view, doc, opts.indent.tab_size));
            max_lines = max_lines.max(doc.buffer.rope().len_lines());
        }
    }
    for dec in [&mut ldec, &mut rdec] {
        dec.width_cols = width_cols;
        dec.max_lines = max_lines;
    }
    let pending = s.sync_pending.take();
    if let Some(p) = pending {
        s.sync = Some(p);
    }
    for (side, r, dec) in [
        (Side::Left, left_rect, ldec),
        (Side::Right, right_rect, rdec),
    ] {
        let pane = s.pane(side);
        let mut opts = core.editor_options(pane.view);
        opts.wrap = WrapMode::Off;
        opts.minimap = None;
        opts.fold_ranges = None;
        opts.max_width_cols = None;
        opts.diff = Some(dec);
        opts.git_changes = None;
        opts.set_scroll_px = pending;
        let mut pui = ui.new_child(egui::UiBuilder::new().max_rect(r));
        pui.set_clip_rect(r);
        let o = editor_widget::show(
            &mut pui,
            kit,
            &mut core.workbench,
            editors,
            pane.view,
            &opts,
            focus_request == Some(pane.view),
            now,
        );
        if o.focused {
            out.focused = true;
            core.workbench.pane_focus.insert(tab, pane.view);
        }
        if o.interacted {
            out.interacted = true;
            core.workbench.pane_focus.insert(tab, pane.view);
        }
        out.zoom_steps += o.zoom_steps;
        out.cjk.merge(o.cjk);
        if let Some(resp) = &o.response {
            resp.context_menu(|ui| {
                ui.set_min_width(200.0);
                let items = menus::visible_items(&pane_context_items(), &core.commands);
                super::chrome::menu_items(ui, kit, core, context, &items, actions);
            });
        }
    }
    // Whichever pane scrolled drives the other next frame.
    if let (Some(a), Some(b)) = (
        editors.scroll_px(s.left.view),
        editors.scroll_px(s.right.view),
    ) {
        let near =
            |x: (f32, f32), y: (f32, f32)| (x.0 - y.0).abs() < 0.5 && (x.1 - y.1).abs() < 0.5;
        match (s.sync, s.driver.take()) {
            (Some(t), Some(Side::Left)) if !near(a, t) => s.sync_pending = Some(a),
            (Some(t), Some(Side::Right)) if !near(b, t) => s.sync_pending = Some(b),
            (None, _) => {
                s.sync = Some(a);
                if !near(a, b) {
                    s.sync_pending = Some(a);
                }
            }
            (Some(t), _) if !near(a, t) => s.sync_pending = Some(a),
            (Some(t), _) if !near(b, t) => s.sync_pending = Some(b),
            _ => {}
        }
        if s.sync_pending.is_some() {
            ui.ctx().request_repaint();
        }
    }

    // Hunk actions toward editable sides.
    ui.painter()
        .rect_filled(middle, 0.0, kit.color(Tok::BgPanel));
    let editable = |side: Side| {
        core.workbench
            .document(s.pane(side).doc)
            .is_some_and(|d| !d.is_read_only())
    };
    let (right_ok, left_ok) = (editable(Side::Right), editable(Side::Left));
    let mut mui = ui.new_child(egui::UiBuilder::new().max_rect(middle));
    mui.set_clip_rect(middle);
    for (i, h) in s.diff.hunks.iter().enumerate() {
        let Some((y, row_h)) = editors.line_y(s.left.view, h.left.start, true) else {
            continue;
        };
        let height = h.left.len().max(h.right.len()).max(1) as f32 * row_h;
        if y + height < middle.top() || y > middle.bottom() {
            continue;
        }
        let band = Rect::from_min_max(
            Pos2::new(middle.left(), y),
            Pos2::new(middle.right(), y + height),
        );
        let current = s.current == Some(i);
        mui.painter().rect_filled(
            band,
            0.0,
            kit.color(if current {
                Tok::AccentSoft
            } else {
                Tok::BgPanelRaised
            }),
        );
        let mut by = y;
        let mut button =
            |ui: &mut egui::Ui, kit: &mut Kit, icon: Icon, tip: String, toward: &str| {
                let r =
                    Rect::from_min_size(Pos2::new(middle.center().x - 11.0, by), Vec2::splat(22.0));
                by += 22.0;
                let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(r));
                if icon_button_sized(&mut bu, kit, icon, &tip, false, true, 22.0).clicked() {
                    actions.push(invoke(
                        "compare.copyHunk",
                        json!({ "view": tab.0, "hunk": i, "toward": toward }),
                    ));
                }
            };
        if right_ok {
            let tip = if s.current_changes {
                t!("compare:revertHunk")
            } else {
                t!("compare:copyToRight")
            };
            button(&mut mui, kit, Icon::ArrowRight, tip, "right");
        }
        if left_ok {
            button(
                &mut mui,
                kit,
                Icon::ArrowLeft,
                t!("compare:copyToLeft"),
                "left",
            );
        }
    }

    // Overview ruler: hunk positions; click jumps to the nearest one.
    ui.painter()
        .rect_filled(ruler, 0.0, kit.color(Tok::BgPanel));
    let total = editors.total_rows(s.left.view).unwrap_or(0).max(1) as f32;
    let mut marks: Vec<(f32, usize)> = Vec::new();
    for (i, h) in s.diff.hunks.iter().enumerate() {
        let Some(row) = editors.line_row(s.left.view, h.left.start, true) else {
            continue;
        };
        let rows = h.left.len().max(h.right.len()).max(1) as f32;
        let y0 = ruler.top() + row as f32 / total * ruler.height();
        let y1 = (y0 + rows / total * ruler.height()).max(y0 + 3.0);
        let color = match (h.left.is_empty(), h.right.is_empty()) {
            (true, _) => kit.color(Tok::DiffGutterInserted),
            (_, true) => kit.color(Tok::DiffGutterRemoved),
            _ => kit.color(Tok::Accent),
        };
        ui.painter().rect_filled(
            Rect::from_min_max(
                Pos2::new(ruler.left() + 3.0, y0),
                Pos2::new(ruler.right() - 3.0, y1),
            ),
            1.0,
            color,
        );
        marks.push((y0, i));
    }
    let resp = ui.interact(ruler, egui::Id::new(("diff-ruler", tab)), Sense::click());
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some(&(_, i)) = marks.iter().min_by(|a, b| {
            (a.0 - p.y)
                .abs()
                .partial_cmp(&(b.0 - p.y).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        actions.push(invoke(
            "compare.revealHunk",
            json!({ "view": tab.0, "hunk": i }),
        ));
    }
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, t!("compare:overview"))
    });

    core.compare.sessions.insert(tab, s);
    out
}

fn pane_context_items() -> Vec<menus::MenuItem> {
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
        MenuItem::Separator,
        cmd("edit.selectAll"),
        MenuItem::Separator,
        cmd("compare.previousChange"),
        cmd("compare.nextChange"),
        cmd("compare.toggleIgnoreWhitespace"),
        cmd("compare.swapSides"),
    ]
}
