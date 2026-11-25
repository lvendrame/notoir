//! Application-level commands: quit, About, notifications (docs/specs/18, 19).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::{KeyBindingDef, Platform};

use super::Specs;
use crate::core::{AppCore, Effect};

pub(super) fn specs(out: &mut Specs, platform: Platform) {
    for (id, handler) in [
        (
            "app.fileAssociations",
            (|c: &mut AppCore, _: &CommandInvocation| {
                c.open_associations();
                CommandOutcome::Done
            }) as notoir_core::command::HandlerFn<AppCore>,
        ),
        (
            "app.saveAssociations",
            |c: &mut AppCore, _: &CommandInvocation| {
                c.associations_request(true);
                CommandOutcome::Done
            },
        ),
        (
            "app.refreshAssociations",
            |c: &mut AppCore, _: &CommandInvocation| {
                c.associations_request(false);
                CommandOutcome::Done
            },
        ),
        (
            "app.cancelAssociations",
            |c: &mut AppCore, _: &CommandInvocation| {
                c.close_associations();
                CommandOutcome::Done
            },
        ),
        (
            "app.selectAssociation",
            |c: &mut AppCore, i: &CommandInvocation| {
                if let (Some(t), Some(s)) = (i.args["type"].as_str(), i.args["selected"].as_bool())
                {
                    c.select_association(t, s);
                }
                CommandOutcome::Done
            },
        ),
    ] {
        let spec = CommandSpec::new(id, Category::Help, handler);
        out.push(if id == "app.fileAssociations" {
            spec
        } else {
            spec.hidden()
        });
    }
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
                c.updates.open = true;
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
    for (id, condition, handler) in [
        (
            "app.addContextMenu",
            "!contextMenuBusy && !contextMenuInstalled",
            add_context as notoir_core::command::HandlerFn<AppCore>,
        ),
        (
            "app.removeContextMenu",
            "!contextMenuBusy && contextMenuInstalled",
            remove_context,
        ),
        ("app.contextMenu", "!contextMenuBusy", show_context),
    ] {
        out.push(
            CommandSpec::new(id, Category::Help, handler)
                .when(condition)
                .expect("valid condition"),
        );
    }
    out.push(
        CommandSpec::new(
            "app.closeContextMenu",
            Category::Help,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.desktop.open = false;
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
    out.push(
        CommandSpec::new(
            "app.closeUpdates",
            Category::Help,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.updates.open = false;
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
    out.push(
        CommandSpec::new(
            "app.quitAndInstall",
            Category::Help,
            |c: &mut AppCore, _: &CommandInvocation| {
                if c.updates.ready.is_some() {
                    c.updates.install_on_quit = true;
                    c.updates.open = false;
                    c.effects.push(Effect::Quit);
                }
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
    for id in ["window.minimize", "window.bringAllToFront", "window.close"] {
        out.push(CommandSpec::new(
            id,
            Category::View,
            |c: &mut AppCore, invocation: &CommandInvocation| {
                let request = match invocation.id.as_str() {
                    "window.minimize" => crate::application::ApplicationRequest::Minimize,
                    "window.bringAllToFront" => {
                        crate::application::ApplicationRequest::BringAllToFront
                    }
                    _ => crate::application::ApplicationRequest::Close,
                };
                c.effects.push(Effect::Application(request));
                CommandOutcome::Done
            },
        ));
    }
    out.push(
        CommandSpec::new(
            "window.activate",
            Category::View,
            |c: &mut AppCore, invocation: &CommandInvocation| {
                if let Some(id) = invocation.args["id"].as_str() {
                    c.effects.push(Effect::Application(
                        crate::application::ApplicationRequest::Activate(id.to_owned()),
                    ));
                }
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
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

fn add_context(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    c.desktop.open = true;
    c.desktop_change(Some(true));
    CommandOutcome::Done
}
fn remove_context(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    c.desktop.open = true;
    c.desktop_change(Some(false));
    CommandOutcome::Done
}
fn show_context(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    c.desktop.open = true;
    c.desktop_change(None);
    CommandOutcome::Done
}
