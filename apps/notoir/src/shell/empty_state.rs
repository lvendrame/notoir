//! Empty editor area (docs/specs/17 §Empty State): wordmark, actions with
//! shortcuts, recent files (pinned first) and the palette hint. Typing here
//! starts a Scratchpad (handled by the app).

use egui::text::LayoutJob;
use egui::{Align, FontId, Layout, RichText, Sense, Vec2};
use notoir_core::command::{CommandId, CommandInvocation, InvocationSource};
use notoir_core::keymap::{KeyChord, Platform};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::theme::{ICON, ICON_SM, Tok};
use crate::{fonts, t};

/// Recent entries shown on the empty state.
const RECENT_ON_EMPTY: usize = 5;

/// Platform-formatted chord for a binding written with `Mod`.
fn shortcut(chord: &str, platform: Platform) -> String {
    KeyChord::parse(chord, platform).map_or_else(|_| chord.to_owned(), |c| c.display(platform))
}

pub fn show(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    let platform = core.platform;
    let (primary, secondary, tertiary) = (
        kit.color(Tok::TextPrimary),
        kit.color(Tok::TextSecondary),
        kit.color(Tok::TextTertiary),
    );
    let width = 360.0_f32.min(ui.available_width());
    ui.add_space((ui.available_height() * 0.22).max(16.0));
    ui.vertical_centered(|ui| {
        let mut wordmark = LayoutJob::default();
        wordmark.append(
            "NOTOIR",
            0.0,
            egui::TextFormat {
                font_id: fonts::extrabold(13.0),
                color: tertiary,
                extra_letter_spacing: 4.0,
                ..Default::default()
            },
        );
        ui.label(wordmark);
        ui.add_space(20.0);
        ui.label(
            RichText::new(t!("welcome:startWriting"))
                .font(fonts::semibold(24.0))
                .color(primary),
        );
        ui.add_space(28.0);

        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
            let rows = [
                (
                    Icon::Plus,
                    "welcome:newScratchpad",
                    "file.newScratchpad",
                    "Mod+Alt+N",
                ),
                (Icon::FolderOpen, "welcome:openFile", "file.open", "Mod+O"),
                (
                    Icon::Folder,
                    "welcome:openWorkspace",
                    "workspace.open",
                    "Mod+K Mod+O",
                ),
            ];
            for (icon, key, command, chord) in rows {
                let enabled = core
                    .commands
                    .get(&CommandId::from(command.to_owned()))
                    .is_some();
                let (rect, resp) = ui.allocate_exact_size(
                    Vec2::new(width, 30.0),
                    if enabled {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                if resp.hovered() && enabled {
                    ui.painter().rect_filled(
                        rect,
                        crate::theme::RADIUS,
                        kit.color(Tok::BgPanelRaised),
                    );
                }
                let color = if enabled { primary } else { tertiary };
                let icon_color = if enabled {
                    kit.color(Tok::Accent)
                } else {
                    tertiary
                };
                kit.icon(ui.ctx(), icon, ICON, icon_color).paint_at(
                    ui,
                    egui::Rect::from_center_size(
                        rect.left_center() + Vec2::new(14.0, 0.0),
                        Vec2::splat(ICON),
                    ),
                );
                ui.painter().text(
                    rect.left_center() + Vec2::new(32.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    t!(key),
                    FontId::proportional(14.0),
                    color,
                );
                ui.painter().text(
                    rect.right_center() - Vec2::new(8.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    shortcut(chord, platform),
                    fonts::mono(12.0),
                    tertiary,
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, t!(key))
                });
                if resp.clicked() {
                    actions.push(CommandInvocation::new(command, InvocationSource::Internal));
                }
            }
            ui.add_space(24.0);
            super::section_label(ui, kit, &t!("welcome:recent"));
            ui.add_space(4.0);
            // Workspaces first, then files; pinned first within each.
            let recent: Vec<(bool, &notoir_storage::state_db::RecentEntry)> = core
                .files
                .recent_workspaces
                .iter()
                .map(|r| (true, r))
                .chain(core.files.recent_files.iter().map(|r| (false, r)))
                .take(RECENT_ON_EMPTY)
                .collect();
            if recent.is_empty() {
                ui.label(
                    RichText::new(t!("welcome:noRecent"))
                        .size(13.0)
                        .color(secondary),
                );
            }
            for (is_workspace, r) in recent {
                let (open_id, pin_id, remove_id) = if is_workspace {
                    (
                        "workspace.openRecent",
                        "workspace.pinRecent",
                        "workspace.removeRecent",
                    )
                } else {
                    ("file.openRecent", "file.pinRecent", "file.removeRecent")
                };
                let path = std::path::Path::new(&r.path);
                let name = path
                    .file_name()
                    .map_or_else(|| r.path.clone(), |n| n.to_string_lossy().into_owned());
                let parent = path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        crate::theme::RADIUS,
                        kit.color(Tok::BgPanelRaised),
                    );
                }
                let mut x = rect.left() + 6.0;
                let lead = if r.pinned {
                    Some(Icon::Pin)
                } else if is_workspace {
                    Some(Icon::Folder)
                } else {
                    None
                };
                if let Some(lead) = lead {
                    kit.icon(ui.ctx(), lead, ICON_SM, tertiary).paint_at(
                        ui,
                        egui::Rect::from_center_size(
                            egui::pos2(x + ICON_SM / 2.0, rect.center().y),
                            Vec2::splat(ICON_SM),
                        ),
                    );
                }
                x += ICON_SM + 8.0;
                let name_galley = ui.painter().layout_no_wrap(
                    name.clone(),
                    FontId::proportional(13.0),
                    kit.color(Tok::Accent),
                );
                let name_w = name_galley.size().x;
                ui.painter().galley(
                    egui::pos2(x, rect.center().y - name_galley.size().y / 2.0),
                    name_galley,
                    kit.color(Tok::Accent),
                );
                ui.painter().text(
                    egui::pos2(x + name_w + 8.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    parent,
                    FontId::proportional(12.0),
                    tertiary,
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone())
                });
                let args = json!({ "path": r.path });
                if resp.clicked() {
                    actions.push(
                        CommandInvocation::new(open_id, InvocationSource::Internal)
                            .with_args(args.clone()),
                    );
                }
                resp.context_menu(|ui| {
                    let (label, pinned) = if r.pinned {
                        (t!("welcome:unpin"), false)
                    } else {
                        (t!("welcome:pin"), true)
                    };
                    if ui.button(label).clicked() {
                        actions.push(
                            CommandInvocation::new(pin_id, InvocationSource::ContextMenu)
                                .with_args(json!({ "path": r.path, "pinned": pinned })),
                        );
                        ui.close();
                    }
                    if ui.button(t!("welcome:removeRecent")).clicked() {
                        actions.push(
                            CommandInvocation::new(remove_id, InvocationSource::ContextMenu)
                                .with_args(args),
                        );
                        ui.close();
                    }
                });
            }
        });

        ui.add_space(40.0);
        let mut hint = LayoutJob::default();
        hint.append(
            &shortcut("Mod+Shift+P", platform),
            0.0,
            egui::TextFormat::simple(fonts::mono(12.0), secondary),
        );
        hint.append(
            &format!(" — {}", t!("welcome:commandPalette")),
            0.0,
            egui::TextFormat::simple(FontId::proportional(12.0), tertiary),
        );
        ui.label(hint);
    });
}
