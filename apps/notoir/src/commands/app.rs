//! Application-level commands: quit, About, notifications (docs/specs/18, 19).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::{KeyBindingDef, Platform};

use super::Specs;
use crate::core::{AppCore, Effect};

pub(super) fn specs(out: &mut Specs, platform: Platform) {
    let mut quit =
        CommandSpec::new("app.quit", Category::File, quit).keys([KeyBindingDef::new("Mod+Q")]);
    if platform == Platform::Mac {
        quit.title_key = "commands:app.quitMac".into();
    }
    out.push(quit);
    out.push(CommandSpec::new("help.about", Category::Help, about).icon("info"));
    out.push(
        CommandSpec::new(
            "workbench.showNotifications",
            Category::View,
            show_notifications,
        )
        .icon("bell"),
    );
    out.push(CommandSpec::new(
        "workbench.clearNotifications",
        Category::View,
        clear_notifications,
    ));
}

fn quit(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::Quit);
    CommandOutcome::Done
}

fn about(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::ShowAbout);
    CommandOutcome::Done
}

fn show_notifications(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::ToggleNotificationCentre);
    CommandOutcome::Done
}

fn clear_notifications(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.notifications.clear();
    CommandOutcome::Done
}
