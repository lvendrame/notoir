//! Activity bar, sidebars and bottom panel (docs/specs/17 §Panel Model).

use egui::{Align, Frame, Layout, Margin, RichText};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::notifications::Severity;
use notoir_core::panels::{PanelArea, PanelId, Side};
use serde_json::json;

use super::{Actions, Kit, icon_button, section_label};
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
        _ => Icon::Files,
    }
}

fn side_panel(id: &'static str, side: Side) -> egui::Panel {
    match side {
        Side::Left => egui::Panel::left(id),
        Side::Right => egui::Panel::right(id),
    }
}

/// Renders every visible area around the editor; call before the central panel.
pub fn show(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) -> PanelSizes {
    let layout = &core.panels;
    let mut sizes = PanelSizes::default();

    if layout.activity_bar_visible && !layout.primary.panels.is_empty() {
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
            .show(ui, |ui| area_contents(ui, kit, core, area, active, actions));
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
                area_contents(ui, kit, core, PanelArea::Bottom, active, actions)
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

fn area_contents(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &AppCore,
    area: PanelArea,
    active: PanelId,
    actions: &mut Actions,
) {
    let state = core.panels.area(area);
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
            if icon_button(ui, kit, Icon::X, &t!("panels:menu.close"), false, true).clicked() {
                actions.push(
                    CommandInvocation::new("view.togglePanel", InvocationSource::Internal)
                        .with_args(json!({ "panel": panel_key(active) })),
                );
            }
            let more = icon_button(
                ui,
                kit,
                Icon::Ellipsis,
                &t!("panels:menu.more"),
                false,
                true,
            );
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(200.0);
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
        PanelId::Explorer => explorer_placeholder(ui, kit),
        PanelId::Output => output(ui, kit, core),
        _ => {}
    }
}

/// Explorer without a workspace (the tree arrives in M7).
fn explorer_placeholder(ui: &mut egui::Ui, kit: &Kit) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(t!("panels:explorer.noFolder"))
            .size(13.0)
            .color(kit.color(Tok::TextSecondary)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("panels:explorer.noFolderHint"))
            .size(12.0)
            .color(kit.color(Tok::TextTertiary)),
    );
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
