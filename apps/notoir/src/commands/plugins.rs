//! Plugin commands (docs/specs/30): reload, install from a folder, review
//! and approval, enable/disable, Plugin Output, and the hidden commands the
//! plugin UI runs. Each plugin's own commands are registered from its
//! manifest (`plugin.<id>.<command>`).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::panels::PanelId;

use super::{Specs, arg, bad_args};
use crate::core::AppCore;

type Handler = fn(&mut AppCore, &CommandInvocation) -> CommandOutcome;

fn done() -> CommandOutcome {
    CommandOutcome::Done
}

fn show_output(core: &mut AppCore) -> CommandOutcome {
    core.exit_maximize();
    core.panels.show(PanelId::PluginOutput);
    done()
}

pub(super) fn specs(out: &mut Specs) {
    let spec = |id: &'static str, f: Handler| CommandSpec::new(id, Category::Plugins, f);
    let hidden = |id: &'static str, f: Handler| CommandSpec::new(id, Category::Plugins, f).hidden();
    out.push(spec("plugins.reload", |c, _| {
        c.plugins_reload();
        done()
    }));
    out.push(spec("plugins.installFromFolder", |c, _| {
        c.plugins_install_from_folder();
        done()
    }));
    out.push(spec("plugins.output", |c, _| show_output(c)));
    out.push(spec("plugins.openFolder", |c, _| {
        let dir = c.plugins_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = notoir_platform::reveal::reveal(&dir) {
            tracing::warn!(error = %e, "could not reveal the plugins folder");
        }
        done()
    }));
    out.push(hidden("plugins.review", |c, i| match arg(i, "plugin") {
        Some(id) if c.plugin_review(id) => done(),
        _ => bad_args(i),
    }));
    out.push(hidden("plugins.reviewAnswer", |c, i| {
        let approve = i.args.get("approve").and_then(|v| v.as_bool()) == Some(true);
        c.plugins_review_answer(approve);
        done()
    }));
    out.push(hidden("plugins.enable", |c, i| match arg(i, "plugin") {
        Some(id) => {
            let id = id.to_owned();
            c.plugin_set_enabled(&id, true);
            done()
        }
        None => bad_args(i),
    }));
    out.push(hidden("plugins.disable", |c, i| match arg(i, "plugin") {
        Some(id) => {
            let id = id.to_owned();
            c.plugin_set_enabled(&id, false);
            done()
        }
        None => bad_args(i),
    }));
    out.push(hidden("plugins.viewError", |c, _| show_output(c)));
    out.push(hidden("plugins.processAnswer", |c, i| {
        match arg(i, "answer") {
            Some(a) => {
                let a = a.to_owned();
                c.plugins_process_answer(&a);
                done()
            }
            None => bad_args(i),
        }
    }));
    out.push(hidden("plugins.run", |c, i| match arg(i, "id") {
        Some(id) if id.starts_with("plugin.") => {
            let id = id.to_owned();
            c.enqueue(CommandInvocation::new(
                id,
                notoir_core::command::InvocationSource::Menu,
            ));
            done()
        }
        _ => bad_args(i),
    }));
    out.push(spec("plugins.manager", |c, _| {
        c.plugins_open_manager();
        done()
    }));
    out.push(spec("plugins.installed", |c, _| {
        c.plugins.manager.tab = crate::plugin_contrib::ManagerTab::Installed;
        c.plugins_open_manager();
        done()
    }));
    out.push(spec("plugins.developerTools", |c, _| show_output(c)));
    out.push(spec("plugins.installFromZip", |c, _| {
        c.plugins_install_from_zip();
        done()
    }));
    out.push(hidden("plugins.uninstall", |c, i| match arg(i, "plugin") {
        Some(id) if c.plugins.entry(id).is_some() => {
            c.plugins.uninstall_confirm = Some(id.to_owned());
            done()
        }
        _ => bad_args(i),
    }));
    out.push(hidden("plugins.uninstallAnswer", |c, i| {
        let approve = i.args.get("approve").and_then(|v| v.as_bool()) == Some(true);
        if let Some(id) = c.plugins.uninstall_confirm.take()
            && approve
        {
            c.plugin_uninstall(&id);
        }
        done()
    }));
    out.push(hidden("plugins.reloadPlugin", |c, i| {
        match arg(i, "plugin") {
            Some(id) => {
                let id = id.to_owned();
                c.plugin_reload_one(&id);
                done()
            }
            None => bad_args(i),
        }
    }));
    out.push(hidden("plugins.panelAction", |c, i| {
        match (arg(i, "plugin"), arg(i, "command")) {
            (Some(p), Some(cmd)) => {
                let (p, cmd) = (p.to_owned(), cmd.to_owned());
                let args = i.args.get("args").cloned().unwrap_or_default();
                c.plugin_action(&p, &cmd, args);
                done()
            }
            _ => bad_args(i),
        }
    }));
    out.push(hidden("plugins.togglePermissions", |c, i| {
        match arg(i, "plugin") {
            Some(id) => {
                let id = id.to_owned();
                let m = &mut c.plugins.manager.expanded;
                *m = if m.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id)
                };
                done()
            }
            None => bad_args(i),
        }
    }));
    out.push(hidden("plugins.revokeProcess", |c, i| {
        match arg(i, "plugin") {
            Some(id) => {
                let id = id.to_owned();
                c.mcp.gate.set(
                    &notoir_core::permissions::Principal::Plugin(id.clone()),
                    notoir_core::permissions::Capability::Process,
                    notoir_core::permissions::Decision::Ask,
                );
                if let Some(db) = &c.state_db {
                    let _ = db.set_plugin_grant(&id, "process", None);
                }
                done()
            }
            None => bad_args(i),
        }
    }));
    out.push(hidden("plugins.settings", |c, i| match arg(i, "plugin") {
        Some(id) => {
            let query = format!("plugins.{id}.");
            c.open_settings(Some(notoir_core::settings::SettingCategory::Plugins));
            c.settings_ui.search = query;
            done()
        }
        None => bad_args(i),
    }));
}
