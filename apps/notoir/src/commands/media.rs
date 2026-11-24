use crate::core::AppCore;
use notoir_core::command::{Category, CommandOutcome, CommandSpec};
use notoir_core::keymap::KeyBindingDef;
pub fn specs(out: &mut super::Specs) {
    for (id, icon) in [
        ("media.playPause", "play"),
        ("media.stop", "square"),
        ("media.mute", "volume-x"),
        ("media.volume", "volume-2"),
        ("media.seek", "play"),
        ("media.zoomIn", "zoom-in"),
        ("media.zoomOut", "zoom-out"),
        ("media.fit", "expand"),
        ("media.actualSize", "scan"),
        ("media.setZoom", "zoom-in"),
        ("media.retry", "refresh-cw"),
    ] {
        let mut spec = CommandSpec::new(id, Category::View, |c: &mut AppCore, i| {
            c.media_action(i);
            CommandOutcome::Done
        })
        .icon(icon);
        if matches!(id, "media.seek" | "media.volume" | "media.setZoom") {
            spec = spec.hidden();
        } else {
            spec = spec.when("mediaActive").expect("valid");
        }
        if id == "media.playPause" {
            spec = spec.keys([KeyBindingDef::new("Space")]);
        }
        out.push(spec);
    }
}
