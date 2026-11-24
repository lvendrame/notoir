//! Accessible themed Lucide controls; narrow splits use separate control rows.
use super::{Actions, Kit, icon_button};
use crate::{core::AppCore, icons::Icon, t};
use notoir_core::{
    command::{CommandInvocation, InvocationSource},
    document::MediaKind,
    ids::ViewId,
};
use serde_json::json;
fn action(actions: &mut Actions, view: ViewId, id: &'static str, value: Option<f64>) {
    actions.push(
        CommandInvocation::new(id, InvocationSource::Toolbar)
            .with_args(json!({"view":view.0,"value":value})),
    );
}
fn volume(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    view: ViewId,
    session: &crate::media::Session,
    actions: &mut Actions,
) {
    let icon = if session.muted || session.volume == 0.0 {
        Icon::VolumeX
    } else {
        Icon::Volume2
    };
    if icon_button(ui, kit, icon, &t!("media:mute"), session.muted, true).clicked() {
        action(actions, view, "media.mute", None);
    }
    let mut volume = session.volume;
    ui.spacing_mut().slider_width = ui.available_width().clamp(20.0, 100.0);
    let r = ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, t!("media:volume")));
    if r.on_hover_text(t!("media:volume")).changed() {
        action(actions, view, "media.volume", Some(volume as f64));
    }
}
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    view: ViewId,
    actions: &mut Actions,
) {
    let Some(v) = core.workbench.view(view) else {
        return;
    };
    let zoom = v.zoom;
    let Some(doc) = core.workbench.document(v.document_id) else {
        return;
    };
    let kind = doc.media_kind();
    let pane = ui.available_rect_before_wrap();
    let width = pane.width();
    let Some(state) = core.media.as_mut() else {
        ui.spinner();
        return;
    };
    let effective_zoom = state.scales.get(&view).copied().unwrap_or(100.0);
    let Some(session) = state.sessions.get_mut(&doc.id) else {
        ui.spinner();
        return;
    };
    ui.push_id(view.0, |ui| {
        if kind == Some(MediaKind::Gif) {
            ui.horizontal(|ui| {
                let (icon, key) = if session.playing {
                    (Icon::Pause, "media:pause")
                } else {
                    (Icon::Play, "media:play")
                };
                if icon_button(ui, kit, icon, &t!(key), false, session.error.is_none()).clicked() {
                    action(actions, view, "media.playPause", None);
                }
            });
        }

        if matches!(kind, Some(MediaKind::Audio | MediaKind::Video)) {
            let time = format!(
                "{} / {}",
                crate::media::time_label(session.position),
                session
                    .duration
                    .map(crate::media::time_label)
                    .unwrap_or_else(|| t!("media:unknownDuration"))
            );
            ui.horizontal(|ui| {
                for (icon, key, id) in [
                    (
                        if session.playing {
                            Icon::Pause
                        } else {
                            Icon::Play
                        },
                        if session.playing {
                            "media:pause"
                        } else {
                            "media:play"
                        },
                        "media.playPause",
                    ),
                    (Icon::Square, "media:stop", "media.stop"),
                ] {
                    if icon_button(ui, kit, icon, &t!(key), false, session.error.is_none())
                        .clicked()
                    {
                        action(actions, view, id, None);
                    }
                }
                ui.spacing_mut().slider_width = if width < 500.0 {
                    ui.available_width().clamp(20.0, 200.0)
                } else {
                    150.0
                };
                let mut position = session.position;
                let enabled = session.duration.is_some() && session.error.is_none();
                let r = ui.add_enabled(
                    enabled,
                    egui::Slider::new(
                        &mut position,
                        0.0..=session.duration.unwrap_or(1.0).max(0.001),
                    )
                    .show_value(false),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Slider, enabled, t!("media:seek"))
                });
                if r.on_hover_text(t!("media:seek")).changed() {
                    action(actions, view, "media.seek", Some(position));
                }
                if width >= 500.0 {
                    ui.monospace(&time);
                    volume(ui, kit, view, session, actions);
                }
            });
            if width < 500.0 {
                ui.monospace(time);
                ui.horizontal(|ui| volume(ui, kit, view, session, actions));
            }
        }
        if kind != Some(MediaKind::Audio) {
            ui.horizontal(|ui| {
                for (icon, key, id) in [
                    (Icon::ZoomOut, "media:zoomOut", "media.zoomOut"),
                    (Icon::ZoomIn, "media:zoomIn", "media.zoomIn"),
                    (Icon::Expand, "media:fit", "media.fit"),
                    (Icon::Scan, "media:actualSize", "media.actualSize"),
                ] {
                    let active = (id == "media.fit" && zoom.is_none())
                        || (id == "media.actualSize" && zoom == Some(100.0));
                    if icon_button(ui, kit, icon, &t!(key), active, true).clicked() {
                        action(actions, view, id, None);
                    }
                }
                if width >= 230.0 {
                    zoom_input(ui, view, zoom.unwrap_or(effective_zoom), actions);
                }
            });
            if width < 230.0 {
                ui.horizontal(|ui| zoom_input(ui, view, zoom.unwrap_or(effective_zoom), actions));
            }
        }
        ui.separator();
        if session.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t!("media:loading"));
            });
        }
        if let Some(error) = &session.error {
            ui.colored_label(ui.visuals().error_fg_color, t!("media:failed"));
            ui.add(
                egui::Label::new(if error.starts_with("media:") {
                    t!(error)
                } else {
                    error.clone()
                })
                .wrap(),
            );
            if icon_button(ui, kit, Icon::RefreshCw, &t!("media:retry"), false, true).clicked() {
                action(actions, view, "media.retry", None);
            }
        }
        if let Some(texture) = &session.texture {
            let canvas =
                egui::Rect::from_min_max(egui::pos2(pane.left(), ui.cursor().top()), pane.max);
            if !canvas.is_positive() {
                return;
            }
            let natural = egui::vec2(session.size[0] as f32, session.size[1] as f32)
                / ui.ctx().pixels_per_point();
            let fit = (canvas.width() / natural.x)
                .min(canvas.height() / natural.y)
                .min(1.0);
            let scale = zoom.map_or(fit, |p| p / 100.0);
            state.scales.insert(view, scale * 100.0);
            let size = natural * scale;
            let mut canvas_ui = ui.new_child(egui::UiBuilder::new().max_rect(canvas));
            canvas_ui.set_clip_rect(canvas);
            egui::ScrollArea::both()
                .id_salt("media-pan")
                .auto_shrink([false, false])
                .show(&mut canvas_ui, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(canvas.size().max(size), egui::Sense::hover());
                    egui::Image::new((texture.id(), size))
                        .paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
                });
        }
    });
}
fn zoom_input(ui: &mut egui::Ui, view: ViewId, zoom: f32, actions: &mut Actions) {
    let mut percent = zoom;
    let r = ui.add(
        egui::DragValue::new(&mut percent)
            .range(10.0..=800.0)
            .suffix("%"),
    );
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, t!("media:zoom"))
    });
    if r.changed() {
        action(actions, view, "media.setZoom", Some(percent as f64));
    }
}
