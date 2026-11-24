use crate::core::AppCore;
use notoir_core::command::{Category, CommandOutcome, CommandSpec};
pub fn specs(out: &mut super::Specs) {
    out.push(
        CommandSpec::new("svg.action", Category::View, |c: &mut AppCore, i| {
            c.svg_zoom_action(i);
            CommandOutcome::Done
        })
        .hidden(),
    );
    for (id, handler) in [
        (
            "svg.showEditor",
            (|c: &mut AppCore, _: &notoir_core::command::CommandInvocation| {
                if let Some(v) = c.workbench.active_view() {
                    c.set_svg_mode(v, "edit");
                }
                CommandOutcome::Done
            }) as notoir_core::command::HandlerFn<AppCore>,
        ),
        ("svg.showSplit", |c, i| mode(c, i, "split")),
        ("svg.showPreview", |c, i| mode(c, i, "preview")),
    ] {
        out.push(
            CommandSpec::new(id, Category::View, handler)
                .when("hasActiveEditor && svgActive")
                .expect("valid"),
        );
    }
    for (id, icon) in [
        ("svg.zoomIn", "zoom-in"),
        ("svg.zoomOut", "zoom-out"),
        ("svg.fit", "expand"),
        ("svg.actualSize", "scan"),
        ("svg.setZoom", "zoom-in"),
        ("svg.retry", "refresh-cw"),
    ] {
        let mut spec = CommandSpec::new(id, Category::View, |c: &mut AppCore, i| {
            c.svg_zoom_action(i);
            CommandOutcome::Done
        })
        .icon(icon);
        if id == "svg.setZoom" {
            spec = spec.hidden();
        } else {
            spec = spec.when("svgActive").expect("valid");
        }
        out.push(spec);
    }
}
fn mode(
    c: &mut AppCore,
    _: &notoir_core::command::CommandInvocation,
    mode: &str,
) -> CommandOutcome {
    if let Some(v) = c.workbench.active_view() {
        c.set_svg_mode(v, mode);
    }
    CommandOutcome::Done
}
