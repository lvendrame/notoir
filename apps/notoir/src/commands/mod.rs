//! Built-in command handlers, one module per area (docs/specs/05). Each
//! module registers its commands; handlers operate on [`AppCore`].

use notoir_core::command::{
    CommandInvocation, CommandOutcome, CommandRegistry, CommandSpec, UserError,
};
use notoir_core::keymap::Platform;

use crate::core::AppCore;

mod app;
mod editor;
mod file;
mod json;
mod layout;
mod markdown;
mod preferences;
mod reliability;
mod settings_ui;
mod structure;
mod text;
mod view;
mod workspace;

pub fn register_all(registry: &mut CommandRegistry<AppCore>, platform: Platform) {
    let mut specs = Vec::new();
    view::specs(&mut specs);
    editor::specs(&mut specs);
    file::specs(&mut specs);
    layout::specs(&mut specs);
    reliability::specs(&mut specs);
    text::specs(&mut specs);
    settings_ui::specs(&mut specs);
    markdown::specs(&mut specs);
    json::specs(&mut specs);
    structure::specs(&mut specs);
    workspace::specs(&mut specs);
    preferences::specs(&mut specs);
    app::specs(&mut specs, platform);
    for spec in specs {
        if let Err(e) = registry.register(spec) {
            tracing::error!(error = %e, "duplicate built-in command");
        }
    }
}

/// String argument `name` of an invocation.
fn arg<'a>(inv: &'a CommandInvocation, name: &str) -> Option<&'a str> {
    inv.args.get(name).and_then(|v| v.as_str())
}

fn bad_args(inv: &CommandInvocation) -> CommandOutcome {
    CommandOutcome::Rejected(UserError::new("errors:invalidCommandArgs").arg("command", &inv.id))
}

type Specs = Vec<CommandSpec<AppCore>>;
