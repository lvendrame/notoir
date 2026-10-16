//! MCP commands (docs/specs/29, 32): server start/stop, Connections,
//! Permissions, Activity and Settings, plus the hidden commands the MCP UI
//! runs (add/revoke clients, edit permissions, answer prompts).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::KeyBindingDef;
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::panels::PanelId;
use notoir_core::permissions::{Capability, ClientId, Decision};
use notoir_core::settings::SettingCategory;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect};
use crate::mcp::{McpDialog, McpTarget, PromptAnswer};

type Handler = fn(&mut AppCore, &CommandInvocation) -> CommandOutcome;

fn done() -> CommandOutcome {
    CommandOutcome::Done
}

pub(super) fn specs(out: &mut Specs) {
    let spec = |id: &'static str, f: Handler| CommandSpec::new(id, Category::Mcp, f);
    let hidden = |id: &'static str, f: Handler| CommandSpec::new(id, Category::Mcp, f).hidden();
    out.push(
        spec("mcp.startServer", |c, _| {
            c.mcp_start();
            done()
        })
        .when("!mcpRunning")
        .expect("valid"),
    );
    out.push(
        spec("mcp.stopServer", |c, _| {
            c.mcp_stop();
            done()
        })
        .when("mcpRunning")
        .expect("valid"),
    );
    out.push(spec("mcp.connections", |c, _| {
        c.mcp.popover = false;
        c.mcp.dialog = Some(McpDialog::Connections);
        done()
    }));
    out.push(spec("mcp.permissions", permissions));
    out.push(spec("mcp.activity", |c, _| {
        c.mcp.popover = false;
        c.exit_maximize();
        c.panels.show(PanelId::McpActivity);
        done()
    }));
    out.push(spec("mcp.settings", |c, _| {
        c.open_settings(Some(SettingCategory::Mcp));
        done()
    }));
    out.push(spec("mcp.addClient", add_client));
    out.push(
        spec("mcp.focusPrompt", |c, _| {
            c.mcp.focus_prompt = true;
            done()
        })
        .keys([KeyBindingDef::new("F6").when("mcpPrompt")])
        .when("mcpPrompt")
        .expect("valid"),
    );
    out.push(hidden("mcp.togglePopover", |c, _| {
        c.mcp.popover = !c.mcp.popover;
        done()
    }));
    out.push(hidden("mcp.closeDialog", |c, _| {
        c.mcp.dialog = None;
        c.mcp.new_client = None;
        done()
    }));
    out.push(hidden("mcp.revokeClient", |c, i| {
        with_client(c, i, |c, id| c.mcp_revoke(&id))
    }));
    out.push(hidden("mcp.resetPermissions", |c, i| {
        with_client(c, i, |c, id| c.mcp_reset_permissions(&id))
    }));
    out.push(hidden("mcp.denyAll", |c, i| {
        with_client(c, i, |c, id| c.mcp_deny_all(&id))
    }));
    out.push(hidden("mcp.clearSessionGrants", |c, i| {
        with_client(c, i, |c, id| {
            c.mcp
                .gate
                .clear_session(&notoir_core::permissions::Principal::McpClient(id))
        })
    }));
    out.push(hidden("mcp.setPermission", set_permission));
    out.push(hidden("mcp.answerPrompt", |c, i| {
        match arg(i, "answer").and_then(PromptAnswer::parse) {
            Some(a) => {
                c.mcp_answer(a);
                done()
            }
            None => bad_args(i),
        }
    }));
    out.push(hidden("mcp.clearActivity", |c, _| {
        c.mcp_clear_activity();
        done()
    }));
    out.push(hidden("mcp.filterActivity", |c, i| {
        c.mcp.activity_filter = arg(i, "client").map(|s| ClientId(s.to_owned()));
        done()
    }));
    out.push(hidden("mcp.copyConfig", copy_config));
    out.push(hidden("mcp.copySetup", copy_setup));
}

fn client_arg(inv: &CommandInvocation) -> Option<ClientId> {
    arg(inv, "client").map(|s| ClientId(s.to_owned()))
}

fn with_client(
    core: &mut AppCore,
    inv: &CommandInvocation,
    f: impl FnOnce(&mut AppCore, ClientId),
) -> CommandOutcome {
    match client_arg(inv).filter(|id| core.mcp.client(id).is_some()) {
        Some(id) => {
            f(core, id);
            done()
        }
        None => bad_args(inv),
    }
}

/// `{ client }` or the first active client.
fn permissions(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    core.mcp.popover = false;
    let id = client_arg(inv).or_else(|| {
        core.mcp
            .clients
            .iter()
            .find(|c| !c.revoked)
            .map(|c| c.id.clone())
    });
    core.mcp.dialog = Some(match id {
        Some(id) => McpDialog::Permissions(id),
        // No clients yet: Connections offers Add Client.
        None => McpDialog::Connections,
    });
    done()
}

/// Without `name`: opens the Add Client form. With it: registers the
/// client and shows its token once.
fn add_client(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    core.mcp.popover = false;
    match arg(inv, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => {
            let name = name.to_owned();
            core.mcp_add_client(&name);
            core.mcp.dialog = Some(McpDialog::Connections);
        }
        None => {
            core.mcp.new_client = None;
            core.mcp.dialog = Some(McpDialog::AddClient(String::new()));
        }
    }
    done()
}

fn set_permission(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let cap = arg(inv, "capability").and_then(Capability::from_key);
    let decision = arg(inv, "decision").and_then(Decision::parse);
    match (client_arg(inv), cap, decision) {
        (Some(id), Some(cap), Some(d)) if core.mcp.client(&id).is_some() => {
            core.mcp_set_permission(&id, cap, d);
            done()
        }
        _ => bad_args(inv),
    }
}

/// `{ client, kind }` (an [`McpTarget`] key): the snippet with the
/// token while it is still shown, else with a placeholder.
fn copy_config(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(id) = client_arg(inv) else {
        return bad_args(inv);
    };
    let token = core
        .mcp
        .new_client
        .as_ref()
        .filter(|n| n.id == id)
        .map_or_else(|| "<token>".to_owned(), |n| n.token.clone());
    let target = arg(inv, "kind")
        .and_then(McpTarget::from_key)
        .unwrap_or(McpTarget::ClaudeCode);
    let text = core.mcp.snippet(target, &token);
    core.effects.push(Effect::CopyText(text));
    done()
}

/// `{ target }`: registers a client named after the platform and copies its
/// setup command with the new token.
fn copy_setup(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(target) = arg(inv, "target").and_then(McpTarget::from_key) else {
        return bad_args(inv);
    };
    let brand = target.brand();
    let taken = |name: &str| {
        core.mcp
            .clients
            .iter()
            .any(|c| !c.revoked && c.name.eq_ignore_ascii_case(name))
    };
    let name = std::iter::once(brand.to_owned())
        .chain((2..).map(|n| format!("{brand} {n}")))
        .find(|n| !taken(n))
        .expect("unbounded");
    let new = core.mcp_add_client(&name);
    let text = core.mcp.snippet(target, &new.token);
    core.effects.push(Effect::CopyText(text));
    core.notify(
        Notification::new(
            Severity::Info,
            Translatable::new("mcp:setup.copied")
                .arg("platform", brand)
                .arg("name", name),
        )
        .source(NotificationSource::Mcp),
    );
    done()
}
