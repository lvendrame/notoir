//! Tab strip of the editor group (docs/specs/04): file-type icon, label with
//! disambiguation, state indicators, close/pin, middle-click close,
//! double-click on empty space for a new file, overflow list and the tab
//! context menu.

use std::collections::HashMap;

use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::document::{Document, DocumentOrigin, ReadOnly};
use notoir_core::ids::{GroupId, ViewId};

use super::layout::TabDrag;
use notoir_core::when::ContextSnapshot;
use serde_json::json;

use super::{Actions, Kit, icon_button};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::menus::{self, MenuItem};
use crate::t;
use crate::theme::{ICON_SM, Tok};

pub const TAB_STRIP_HEIGHT: f32 = 34.0;

fn file_icon(doc: &Document) -> Icon {
    language_icon(doc.language.as_str())
}

/// File icon for a language id (tabs, Explorer).
pub fn language_icon(language: &str) -> Icon {
    match language {
        "markdown" | "plaintext" | "log" => Icon::FileText,
        "json" | "jsonc" | "jsonl" => Icon::FileJson,
        "shellscript" | "powershell" => Icon::FileTerminal,
        "toml" | "yaml" | "ini" | "dotenv" | "xml" => Icon::FileCog,
        "rust" | "python" | "javascript" | "typescript" | "html" | "css" | "sql" => Icon::FileCode,
        _ => Icon::File,
    }
}

/// Tab labels for `views`, adding "— parent" where file names collide.
pub fn labels(core: &AppCore, views: &[ViewId]) -> HashMap<ViewId, String> {
    let wb = &core.workbench;
    let mut base: HashMap<ViewId, String> = HashMap::new();
    let mut counts: HashMap<String, Vec<std::path::PathBuf>> = HashMap::new();
    for &v in views {
        let Some(doc) = wb.view(v).and_then(|v| wb.document(v.document_id)) else {
            continue;
        };
        let title = core.document_title(doc.id);
        if let Some(p) = doc.path() {
            let entry = counts.entry(title.clone()).or_default();
            if !entry.iter().any(|q| q == p) {
                entry.push(p.to_path_buf());
            }
        }
        base.insert(v, title);
    }
    let mut out = HashMap::new();
    for (&v, title) in &base {
        let doc = wb.view(v).and_then(|v| wb.document(v.document_id));
        let mut label = title.clone();
        if let Some(path) = doc.and_then(|d| d.path())
            && counts.get(title).is_some_and(|c| c.len() > 1)
            && let Some(parent) = path.parent().and_then(|p| p.file_name())
        {
            label = format!("{title} — {}", parent.to_string_lossy());
        }
        // Duplicate views of one document in the same group: "⧉2".
        let group = wb.layout.group_of(v);
        let doc_id = wb.view(v).map(|v| v.document_id);
        let same: Vec<ViewId> = views
            .iter()
            .copied()
            .filter(|o| {
                wb.layout.group_of(*o) == group && wb.view(*o).map(|x| x.document_id) == doc_id
            })
            .collect();
        if same.len() > 1
            && let Some(i) = same.iter().position(|o| *o == v).filter(|i| *i > 0)
        {
            label = format!("{label} ⧉{}", i + 1);
        }
        out.insert(v, label);
    }
    out
}

fn context_items(core: &AppCore, view: ViewId) -> Vec<MenuItem> {
    let args = json!({ "view": view.0 });
    let cmd = |id: &'static str| MenuItem::Command {
        id,
        args: args.clone(),
        label: None,
        checked: None,
    };
    let pinned = core.workbench.view(view).is_some_and(|v| v.pinned);
    let pin_label = if pinned { "tabs:unpin" } else { "tabs:pin" };
    let reveal_label = notoir_platform::reveal_label_key();
    vec![
        cmd("file.save"),
        cmd("file.saveAs"),
        MenuItem::Separator,
        cmd("file.close"),
        cmd("file.closeOthers"),
        cmd("file.closeToRight"),
        cmd("file.closeSaved"),
        cmd("file.closeAll"),
        MenuItem::Separator,
        cmd("view.duplicate"),
        MenuItem::Separator,
        cmd("compare.with"),
        cmd("compare.select"),
        MenuItem::Separator,
        cmd("view.splitLeft"),
        cmd("view.splitRight"),
        cmd("view.splitUp"),
        cmd("view.splitDown"),
        MenuItem::Separator,
        cmd("workbench.moveTabToPreviousGroup"),
        cmd("workbench.moveTabToNextGroup"),
        MenuItem::Separator,
        MenuItem::Command {
            id: "view.togglePin",
            args: args.clone(),
            label: Some(pin_label.to_owned()),
            checked: None,
        },
        MenuItem::Separator,
        cmd("file.copyPath"),
        cmd("file.copyRelativePath"),
        MenuItem::Command {
            id: "file.reveal",
            args,
            label: Some(reveal_label.to_owned()),
            checked: None,
        },
    ]
}

fn invoke(actions: &mut Actions, id: &'static str, view: ViewId) {
    actions.push(
        CommandInvocation::new(id, InvocationSource::Internal).with_args(json!({ "view": view.0 })),
    );
}

/// Draws one group's tab strip. Tab rectangles are reported for drop targets;
/// dragging a tab starts a [`TabDrag`].
#[allow(clippy::too_many_arguments)]
pub fn strip(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    group_id: GroupId,
    context: &ContextSnapshot,
    actions: &mut Actions,
    drag: &mut Option<TabDrag>,
    tab_rects: &mut Vec<(ViewId, Rect)>,
) -> Rect {
    let Some(group) = core.workbench.layout.group(group_id) else {
        return Rect::NOTHING;
    };
    let focused_group = core.workbench.layout.active_group == group_id;
    let tabs = group.tabs.clone();
    let active = group.active;
    let (rect, bg_resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), TAB_STRIP_HEIGHT),
        Sense::click(),
    );
    ui.painter().rect_filled(rect, 0.0, kit.color(Tok::BgPanel));
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, kit.color(Tok::Border)),
    );
    if bg_resp.double_clicked() {
        actions.push(
            CommandInvocation::new("workbench.focusGroup", InvocationSource::Internal)
                .with_args(json!({ "group": group_id.0 })),
        );
        actions.push(CommandInvocation::new(
            "file.newUntitled",
            InvocationSource::Internal,
        ));
    }
    let labels = labels(core, &tabs);
    let overflow_w = 30.0;
    let strip_rect = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.right() - overflow_w, rect.bottom()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(strip_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    egui::ScrollArea::horizontal()
        .id_salt(("tab-strip", group_id))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                for &view in &tabs {
                    let tab_rect = tab(
                        ui,
                        kit,
                        core,
                        context,
                        view,
                        labels.get(&view).cloned().unwrap_or_default(),
                        Some(view) == active,
                        focused_group,
                        actions,
                        drag,
                        group_id,
                    );
                    tab_rects.push((view, tab_rect));
                }
            });
        });
    // Overflow: list of every tab.
    let mut over = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                Pos2::new(rect.right() - overflow_w, rect.top()),
                rect.max,
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let btn = icon_button(
        &mut over,
        kit,
        Icon::ChevronDown,
        &t!("tabs:showAll"),
        false,
        !tabs.is_empty(),
    );
    egui::Popup::menu(&btn).show(|ui| {
        ui.set_min_width(220.0);
        for &view in &tabs {
            let label = labels.get(&view).cloned().unwrap_or_default();
            if ui.selectable_label(Some(view) == active, label).clicked() {
                invoke(actions, "view.activate", view);
            }
        }
    });
    rect
}

#[allow(clippy::too_many_arguments)]
fn tab(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    context: &ContextSnapshot,
    view: ViewId,
    label: String,
    active: bool,
    focused_group: bool,
    actions: &mut Actions,
    drag: &mut Option<TabDrag>,
    group: GroupId,
) -> Rect {
    let Some(v) = core.workbench.view(view) else {
        return Rect::NOTHING;
    };
    let Some(doc) = core.workbench.document(v.document_id) else {
        return Rect::NOTHING;
    };
    let font = egui::FontId::proportional(13.0);
    let text_color = if active {
        kit.color(Tok::TextPrimary)
    } else {
        kit.color(Tok::TextSecondary)
    };
    let galley = ui.painter().layout_no_wrap(label.clone(), font, text_color);
    let lock = doc.read_only != ReadOnly::No;
    let width = 12.0
        + ICON_SM
        + 6.0
        + if lock { ICON_SM + 4.0 } else { 0.0 }
        + galley.size().x
        + 8.0
        + 20.0
        + 8.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, TAB_STRIP_HEIGHT), Sense::click_and_drag());
    if resp.drag_started() {
        *drag = Some(TabDrag { view, from: group });
    }
    let resp = resp.on_hover_text(match &doc.origin {
        DocumentOrigin::File { path, .. } => path.display().to_string(),
        DocumentOrigin::Scratchpad { .. } => t!("tabs:scratchTooltip"),
        _ => label.clone(),
    });
    let painter = ui.painter_at(rect);
    let hovered = resp.hovered();
    if active {
        painter.rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
        painter.line_segment(
            [
                rect.left_top() + Vec2::new(0.0, 1.0),
                rect.right_top() + Vec2::new(0.0, 1.0),
            ],
            // The focused group's active tab gets the full accent.
            Stroke::new(
                2.0,
                if focused_group {
                    kit.color(Tok::Accent)
                } else {
                    kit.color(Tok::Accent).gamma_multiply(0.45)
                },
            ),
        );
    } else if hovered {
        painter.rect_filled(rect, 0.0, kit.color(Tok::BgPanelRaised));
    }
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0, kit.color(Tok::Border)),
    );
    let mut x = rect.left() + 12.0;
    let icon_rect = |x: f32| {
        Rect::from_center_size(
            Pos2::new(x + ICON_SM / 2.0, rect.center().y),
            Vec2::splat(ICON_SM),
        )
    };
    kit.icon(
        ui.ctx(),
        file_icon(doc),
        ICON_SM,
        kit.color(Tok::TextTertiary),
    )
    .paint_at(ui, icon_rect(x));
    x += ICON_SM + 6.0;
    if lock {
        kit.icon(ui.ctx(), Icon::Lock, ICON_SM, kit.color(Tok::TextTertiary))
            .paint_at(ui, icon_rect(x));
        x += ICON_SM + 4.0;
    }
    painter.galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );

    // Trailing slot: pin, close (hover/active) or dirty dot.
    let slot = Rect::from_center_size(
        Pos2::new(rect.right() - 18.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let slot_resp = ui.interact(slot, ui.id().with(("tab-close", view)), Sense::click());
    let dirty = doc.is_dirty();
    if v.pinned {
        kit.icon(ui.ctx(), Icon::Pin, ICON_SM, kit.color(Tok::TextTertiary))
            .paint_at(ui, slot.shrink(2.0));
        if slot_resp.clicked() {
            invoke(actions, "view.togglePin", view);
        }
    } else if hovered || slot_resp.hovered() || (active && !dirty) {
        if slot_resp.hovered() {
            painter.rect_filled(slot, 3.0, kit.color(Tok::BgPanelRaised));
        }
        kit.icon(ui.ctx(), Icon::X, ICON_SM, kit.color(Tok::TextSecondary))
            .paint_at(ui, slot.shrink(2.0));
        if slot_resp.clicked() {
            invoke(actions, "file.close", view);
        }
    } else if dirty {
        painter.circle_filled(slot.center(), 4.0, kit.color(Tok::TextSecondary));
    }
    if dirty {
        // Accessible state for screen readers and tests.
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{label} ●"))
        });
    } else {
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone())
        });
    }

    if resp.clicked() && !slot_resp.clicked() {
        invoke(actions, "view.activate", view);
    }
    if resp.middle_clicked() {
        invoke(actions, "file.close", view);
    }
    resp.context_menu(|ui| {
        ui.set_min_width(240.0);
        let items = menus::visible_items(&context_items(core, view), &core.commands);
        super::chrome::menu_items(ui, kit, core, context, &items, actions);
    });
    rect
}
