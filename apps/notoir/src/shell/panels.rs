//! Activity bar, sidebars and bottom panel (docs/specs/17 §Panel Model).

use egui::{Align, Frame, Layout, Margin, RichText};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::notifications::Severity;
use notoir_core::panels::{PanelArea, PanelId, Side};
use serde_json::json;

use super::{Actions, Kit, icon_button, icon_button_sized, section_label};
use crate::core::{AppCore, panel_key};
use crate::icons::Icon;
use crate::t;
use crate::theme::Tok;

pub const ACTIVITY_BAR_WIDTH: f32 = 40.0;

/// Sizes the user dragged this frame (written back to the layout).
#[derive(Default)]
pub struct PanelSizes {
    pub primary: Option<f32>,
    pub secondary: Option<f32>,
    pub bottom: Option<f32>,
}

fn activity_icon(p: PanelId) -> Icon {
    match p {
        PanelId::Explorer => Icon::Files,
        PanelId::Search => Icon::Search,
        PanelId::SourceControl => Icon::GitBranch,
        PanelId::Outline => Icon::ListTree,
        PanelId::Plugins => Icon::Puzzle,
        PanelId::McpActivity => Icon::Activity,
        _ => Icon::Files,
    }
}

fn side_panel(id: &'static str, side: Side) -> egui::Panel {
    match side {
        Side::Left => egui::Panel::left(id),
        Side::Right => egui::Panel::right(id),
    }
}

/// Per-panel UI state.
pub struct PanelUis<'a> {
    pub explorer: &'a mut super::explorer::ExplorerUi,
    pub outline: &'a mut super::outline::OutlineUi,
    pub scm: &'a mut super::scm_panel::ScmUi,
}

/// Renders every visible area around the editor; call before the central panel.
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    context: &notoir_core::when::ContextSnapshot,
    uis: &mut PanelUis,
    actions: &mut Actions,
) -> PanelSizes {
    let layout = core.panels.clone();
    let layout = &layout;
    let mut sizes = PanelSizes::default();

    if layout.activity_bar_visible && !layout.primary.panels.is_empty() && core.zen.is_none() {
        side_panel("activity-bar", layout.primary_side)
            .exact_size(ACTIVITY_BAR_WIDTH)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(kit.color(Tok::BgApp))
                    .inner_margin(Margin::symmetric(6, 6)),
            )
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    for &p in &layout.primary.panels {
                        let active = layout.primary.visible && layout.primary.active == Some(p);
                        if icon_button(ui, kit, activity_icon(p), &t!(p.title_key()), active, true)
                            .clicked()
                        {
                            actions.push(
                                CommandInvocation::new(
                                    "view.togglePanel",
                                    InvocationSource::Internal,
                                )
                                .with_args(json!({ "panel": panel_key(p) })),
                            );
                        }
                    }
                });
            });
    }

    for (area, id, side) in [
        (PanelArea::Primary, "primary-sidebar", layout.primary_side),
        (
            PanelArea::Secondary,
            "secondary-sidebar",
            layout.secondary_side(),
        ),
    ] {
        let state = layout.area(area);
        let Some(active) = state.active.filter(|_| state.visible) else {
            continue;
        };
        let resp = side_panel(id, side)
            .resizable(true)
            .default_size(state.size)
            .min_size(area.min_size())
            .frame(panel_frame(kit))
            .show(ui, |ui| {
                area_contents(ui, kit, core, context, uis, area, active, actions)
            });
        let width = resp.response.rect.width();
        if (width - state.size).abs() > 0.5 {
            match area {
                PanelArea::Primary => sizes.primary = Some(width),
                _ => sizes.secondary = Some(width),
            }
        }
    }

    let bottom = layout.area(PanelArea::Bottom);
    if let Some(active) = bottom.active.filter(|_| bottom.visible) {
        let resp = egui::Panel::bottom("bottom-panel")
            .resizable(true)
            .default_size(bottom.size)
            .min_size(PanelArea::Bottom.min_size())
            .frame(panel_frame(kit))
            .show(ui, |ui| {
                area_contents(
                    ui,
                    kit,
                    core,
                    context,
                    uis,
                    PanelArea::Bottom,
                    active,
                    actions,
                )
            });
        let height = resp.response.rect.height();
        if (height - bottom.size).abs() > 0.5 {
            sizes.bottom = Some(height);
        }
    }
    sizes
}

fn panel_frame(kit: &Kit) -> Frame {
    Frame::new()
        .fill(kit.color(Tok::BgPanel))
        .inner_margin(Margin::symmetric(10, 6))
}

#[allow(clippy::too_many_arguments)]
fn area_contents(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    context: &notoir_core::when::ContextSnapshot,
    uis: &mut PanelUis,
    area: PanelArea,
    active: PanelId,
    actions: &mut Actions,
) {
    let state = core.panels.area(area).clone();
    let panel_hovered = ui.rect_contains_pointer(ui.max_rect());
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        if area == PanelArea::Bottom || state.panels.len() > 1 && area == PanelArea::Secondary {
            // Tabs for areas without an activity bar.
            for &p in &state.panels {
                let selected = p == active;
                let text = RichText::new(t!(p.title_key()).to_uppercase())
                    .font(crate::fonts::extrabold(11.0))
                    .color(kit.color(if selected {
                        Tok::TextPrimary
                    } else {
                        Tok::TextTertiary
                    }));
                if ui.add(egui::Button::new(text).frame(false)).clicked() && !selected {
                    actions.push(
                        CommandInvocation::new("view.togglePanel", InvocationSource::Internal)
                            .with_args(json!({ "panel": panel_key(p) })),
                    );
                }
            }
        } else {
            section_label(ui, kit, &t!(active.title_key()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button_sized(
                ui,
                kit,
                Icon::X,
                &t!("panels:menu.close"),
                false,
                true,
                24.0,
            )
            .clicked()
            {
                actions.push(
                    CommandInvocation::new("view.togglePanel", InvocationSource::Internal)
                        .with_args(json!({ "panel": panel_key(active) })),
                );
            }
            let more = icon_button_sized(
                ui,
                kit,
                Icon::Ellipsis,
                &t!("panels:menu.more"),
                false,
                true,
                24.0,
            );
            if active == PanelId::Explorer && panel_hovered {
                super::explorer::toolbar(ui, kit, core, actions);
            }
            if active == PanelId::SourceControl {
                super::scm_panel::toolbar(ui, kit, core, actions);
            }
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(200.0);
                let more_items = match active {
                    PanelId::Explorer => Some(super::explorer::more_menu_items()),
                    PanelId::SourceControl if core.git.repo.is_some() => {
                        Some(super::scm_panel::more_menu_items())
                    }
                    _ => None,
                };
                if let Some(items) = more_items {
                    let items = crate::menus::visible_items(&items, &core.commands);
                    super::chrome::menu_items(ui, kit, core, context, &items, actions);
                    ui.separator();
                }
                for (target, key) in [
                    (PanelArea::Primary, "panels:menu.moveToPrimary"),
                    (PanelArea::Secondary, "panels:menu.moveToSecondary"),
                    (PanelArea::Bottom, "panels:menu.moveToBottom"),
                ] {
                    if target != area
                        && active.allowed_areas().contains(&target)
                        && ui.button(t!(key)).clicked()
                    {
                        let area_key = match target {
                            PanelArea::Primary => "primary",
                            PanelArea::Secondary => "secondary",
                            PanelArea::Bottom => "bottom",
                        };
                        actions.push(
                            CommandInvocation::new("view.movePanel", InvocationSource::ContextMenu)
                                .with_args(json!({ "panel": panel_key(active), "area": area_key })),
                        );
                    }
                }
                ui.separator();
                if ui.button(t!("panels:menu.hide")).clicked() {
                    actions.push(
                        CommandInvocation::new("view.togglePanel", InvocationSource::ContextMenu)
                            .with_args(json!({ "panel": panel_key(active) })),
                    );
                }
            });
        });
    });
    ui.add_space(4.0);
    match active {
        PanelId::Explorer => super::explorer::show(ui, kit, core, context, uis.explorer, actions),
        PanelId::Outline => super::outline::show(ui, kit, core, uis.outline, actions),
        PanelId::Search => super::search_panel::show(ui, kit, core, actions),
        PanelId::Output => output(ui, kit, core),
        PanelId::SourceControl => super::scm_panel::show(ui, kit, core, uis.scm, actions),
        PanelId::GitOutput => super::scm_panel::output(ui, kit, core),
        PanelId::McpActivity => super::mcp_ui::activity_panel(ui, kit, core, actions),
        PanelId::PluginOutput => super::plugins_ui::dev_tools(ui, kit, core, actions),
        PanelId::Plugins => super::plugins_ui::plugins_panel(ui, kit, core, actions),
        _ => {}
    }
}

/// Output (Notoir channel): this session's notifications, newest last.
fn output(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore) {
    let items: Vec<_> = core.notifications.history().collect();
    if items.is_empty() {
        ui.label(
            RichText::new(t!("panels:output.empty"))
                .size(12.0)
                .color(kit.color(Tok::TextTertiary)),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for n in items.iter().rev() {
                let color = match n.severity {
                    Severity::Error => kit.color(Tok::Danger),
                    Severity::Warning => kit.color(Tok::Warning),
                    _ => kit.color(Tok::TextSecondary),
                };
                ui.label(
                    RichText::new(super::overlays::message_text(n))
                        .font(crate::fonts::mono(12.0))
                        .color(color),
                );
            }
        });
}
