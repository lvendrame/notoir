//! First-run / Help → Welcome page (docs/specs/17 §First-Run Experience):
//! language, appearance with live preview, pointers to Git, MCP and plugins.
//! Choices apply immediately; Skip or Start writing finishes it.

use egui::{Align, Layout, RichText, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::i18n::Language;
use crate::t;
use crate::theme::{ThemeId, Tok};

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

pub fn show(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let width = 560.0_f32.min(rect.width() - 40.0);
    let area = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 40.0 + 230.0),
        Vec2::new(width, 460.0),
    );
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(Layout::top_down(Align::Min)),
    );
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(t!("welcome:title"))
                .font(crate::fonts::semibold(24.0))
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("welcome:skip")).clicked() {
                cmd(actions, "welcome.finish", json!({}));
            }
        });
    });
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(t!("welcome:languageLabel"))
                .size(14.0)
                .color(kit.color(Tok::TextSecondary)),
        );
        ui.add_space(12.0);
        egui::ComboBox::from_id_salt("welcome-language")
            .selected_text(core.language.native_label())
            .show_ui(ui, |ui| {
                for l in Language::ALL {
                    if ui
                        .selectable_label(l == core.language, l.native_label())
                        .clicked()
                    {
                        cmd(
                            actions,
                            "preferences.changeLanguage",
                            json!({ "language": l.code() }),
                        );
                    }
                }
            });
    });
    ui.add_space(20.0);
    ui.label(
        RichText::new(t!("welcome:appearance"))
            .size(14.0)
            .color(kit.color(Tok::TextSecondary)),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        for theme in [ThemeId::Paper, ThemeId::Midnight, ThemeId::Dracula] {
            let t = theme.load();
            let selected = t.name == kit.theme.name;
            let (r, resp) = ui.allocate_exact_size(Vec2::new(150.0, 96.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(r, 6.0, t.color(Tok::BgApp));
            let inner = r.shrink(10.0);
            p.rect_filled(
                egui::Rect::from_min_size(inner.min, Vec2::new(inner.width(), 44.0)),
                4.0,
                t.color(Tok::BgEditor),
            );
            for (i, w) in [0.8, 0.55, 0.7].iter().enumerate() {
                let y = inner.top() + 10.0 + i as f32 * 10.0;
                p.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(inner.left() + 8.0, y),
                        Vec2::new((inner.width() - 16.0) * w, 4.0),
                    ),
                    2.0,
                    t.color(if i == 0 {
                        Tok::Accent
                    } else {
                        Tok::TextSecondary
                    }),
                );
            }
            p.text(
                egui::pos2(inner.left(), inner.bottom() - 8.0),
                egui::Align2::LEFT_CENTER,
                &t.name,
                egui::FontId::proportional(13.0),
                t.color(Tok::TextPrimary),
            );
            let stroke = if selected {
                Stroke::new(2.0, kit.color(Tok::Accent))
            } else {
                Stroke::new(1.0, kit.color(Tok::Border))
            };
            p.rect_stroke(r, 6.0, stroke, egui::StrokeKind::Inside);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, &t.name)
            });
            if resp.clicked() {
                cmd(
                    actions,
                    "preferences.changeTheme",
                    json!({ "theme": theme.id() }),
                );
            }
            ui.add_space(8.0);
        }
    });
    ui.add_space(24.0);
    for (label, button, action) in [
        (
            "welcome:git",
            "welcome:configure",
            Some(("app.settings", json!({ "category": "git" }))),
        ),
        (
            "welcome:mcp",
            "welcome:learnMore",
            Some(("mcp.connections", json!({}))),
        ),
        (
            "welcome:plugins",
            "welcome:explore",
            Some(("plugins.manager", json!({}))),
        ),
    ] {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(90.0, 24.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_width(90.0);
                    ui.label(
                        RichText::new(t!(label))
                            .size(14.0)
                            .color(kit.color(Tok::TextSecondary)),
                    );
                },
            );
            match action {
                Some((id, args)) => {
                    if ui.button(t!(button)).clicked() {
                        cmd(actions, id, args);
                    }
                }
                // Plugins arrive in a later milestone.
                None => {
                    ui.add_enabled(false, egui::Button::new(t!(button)))
                        .on_disabled_hover_text(t!("welcome:comingLater"));
                }
            }
        });
        ui.add_space(6.0);
    }
    ui.add_space(24.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let start =
            egui::Button::new(RichText::new(t!("welcome:start")).color(kit.color(Tok::BgApp)))
                .fill(kit.color(Tok::Accent))
                .min_size(Vec2::new(140.0, 32.0));
        if ui.add(start).clicked() {
            cmd(actions, "welcome.finish", json!({}));
        }
    });
}
