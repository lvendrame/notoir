//! Application-level commands: quit, About, notifications (docs/specs/18, 19).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::{KeyBindingDef, Platform};

use super::Specs;
use crate::core::{AppCore, Effect};

pub(super) fn specs(out: &mut Specs, platform: Platform) {
    out.push(CommandSpec::new(
        "app.checkForUpdates",
        Category::Help,
        |c: &mut AppCore, _: &CommandInvocation| {
            c.updates_check(true);
            CommandOutcome::Done
        },
    ));
    out.push(
        CommandSpec::new(
            "app.downloadUpdate",
            Category::Help,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.updates_download();
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
    out.push(
        CommandSpec::new(
            "app.installUpdateOnQuit",
            Category::Help,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.updates_install_on_quit();
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
    // `notoir` on PATH (spec 38): off the UI thread (may ask for admin).
    out.push(CommandSpec::new(
        "app.installShellCommand",
        Category::Help,
        |c: &mut AppCore, _: &CommandInvocation| {
            c.io.submit(|| {
                Some(crate::io::IoResult::ShellCommand {
                    install: true,
                    result: notoir_platform::shell_command::install()
                        .map(|p| p.display().to_string()),
                })
            });
            CommandOutcome::Done
        },
    ));
    out.push(CommandSpec::new(
        "app.uninstallShellCommand",
        Category::Help,
        |c: &mut AppCore, _: &CommandInvocation| {
            c.io.submit(|| {
                Some(crate::io::IoResult::ShellCommand {
                    install: false,
                    result: notoir_platform::shell_command::uninstall()
                        .map(|p| p.display().to_string()),
                })
            });
            CommandOutcome::Done
        },
    ));
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
