//! SVG presentation only; controls dispatch commands and source remains editable.
use super::{Actions, Kit, icon_button};
use crate::{core::AppCore, icons::Icon, t};
use notoir_core::{
    command::{CommandInvocation, InvocationSource},
    ids::ViewId,
};
use serde_json::json;
fn action(actions: &mut Actions, view: ViewId, id: &'static str, value: Option<f64>) {
    actions.push(
        CommandInvocation::new("svg.action", InvocationSource::Toolbar)
            .with_args(json!({"view":view.0,"value":value,"command":id})),
    );
}
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    view: ViewId,
    actions: &mut Actions,
) {
    let pane = ui.available_rect_before_wrap();
    let zoom = core
        .workbench
        .view(view)
        .and_then(|v| v.svg.as_ref())
        .and_then(|s| s.zoom);
    let effective = core
        .svg
        .as_ref()
        .and_then(|s| s.views.get(&view))
        .map_or(100.0, |p| p.effective_zoom);
    ui.push_id(("svg-preview", view.0), |ui| {
        ui.horizontal(|ui| {
            for (icon, key, id) in [
                (Icon::ZoomOut, "media:zoomOut", "svg.zoomOut"),
                (Icon::ZoomIn, "media:zoomIn", "svg.zoomIn"),
                (Icon::Expand, "media:fit", "svg.fit"),
                (Icon::Scan, "media:actualSize", "svg.actualSize"),
            ] {
                let active = (id == "svg.fit" && zoom.is_none())
                    || (id == "svg.actualSize" && zoom == Some(100.0));
                if icon_button(ui, kit, icon, &t!(key), active, true).clicked() {
                    action(actions, view, id, None);
                }
            }
            if pane.width() >= 230.0 {
                zoom_input(ui, view, zoom.unwrap_or(effective), actions);
            }
        });
        if pane.width() < 230.0 {
            ui.horizontal(|ui| zoom_input(ui, view, zoom.unwrap_or(effective), actions));
        }
        ui.separator();
        let area = egui::Rect::from_min_max(egui::pos2(pane.left(), ui.cursor().top()), pane.max);
        if !area.is_positive() {
            return;
        }
        core.prepare_svg(ui.ctx(), view, area.size());
        let Some(preview) = core.svg.as_mut().and_then(|s| s.views.get_mut(&view)) else {
            return;
        };
        if let Some(raster) = preview.ready.take() {
            if raster
                .size
                .iter()
                .any(|s| *s > ui.input(|i| i.max_texture_side))
            {
                preview.error = Some("svg:tooLarge".into());
            } else {
                let image = egui::ColorImage::from_rgba_premultiplied(raster.size, &raster.rgba);
                if let Some(texture) = &mut preview.texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    preview.texture = Some(ui.ctx().load_texture(
                        format!("svg-{}", view.0),
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                preview.natural = raster.natural;
                preview.error = None;
                preview.accept_raster(raster);
            }
        }
        let mut canvas = area;
        if let Some(error) = &preview.error {
            ui.colored_label(ui.visuals().error_fg_color, t!("svg:invalid"));
            ui.add(
                egui::Label::new(if error.starts_with("svg:") {
                    t!(error)
                } else {
                    error.clone()
                })
                .wrap(),
            );
            if preview.texture.is_some() {
                ui.label(t!("svg:lastValid"));
            }
            if icon_button(ui, kit, Icon::RefreshCw, &t!("media:retry"), false, true).clicked() {
                action(actions, view, "svg.retry", None);
            }
            canvas.min.y = ui.cursor().top();
        } else if preview.texture.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t!("svg:rendering"));
            });
            return;
        }
        if !canvas.is_positive() {
            return;
        }
        if let Some(texture) = &preview.texture {
            let natural = egui::vec2(preview.natural[0], preview.natural[1]);
            let fit = (canvas.width() / natural.x)
                .min(canvas.height() / natural.y)
                .min(1.0);
            let scale = zoom.map_or(fit, |z| z / 100.0);
            preview.effective_zoom = scale * 100.0;
            let size = natural * scale;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(canvas));
            child.set_clip_rect(canvas);
            egui::ScrollArea::both()
                .id_salt("svg-pan")
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(canvas.size().max(size), egui::Sense::hover());
                    egui::Image::new((texture.id(), size))
                        .paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
                });
        }
    });
}
fn zoom_input(ui: &mut egui::Ui, view: ViewId, mut percent: f32, actions: &mut Actions) {
    let r = ui.add(
        egui::DragValue::new(&mut percent)
            .range(10.0..=800.0)
            .suffix("%"),
    );
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, t!("media:zoom"))
    });
    if r.changed() {
        action(actions, view, "svg.setZoom", Some(percent as f64));
    }
}
