//! Settings tab, Welcome page, Zen Mode and indentation commands
//! (docs/specs/14, 17, 19, 20).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::KeyBindingDef;
use notoir_core::settings::SettingCategory;
use notoir_editor::edit_ops::IndentStyle;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect};
use crate::shell::overlays::PickerKind;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    out.push(
        CommandSpec::new("app.settings", Category::Preferences, settings)
            .keys([k("Mod+,")])
            .icon("settings"),
    );
    out.push(
        CommandSpec::new(
            "preferences.keyboardShortcuts",
            Category::Preferences,
            shortcuts,
        )
        .icon("keyboard"),
    );
    out.push(CommandSpec::new("help.welcome", Category::Help, welcome));
    out.push(CommandSpec::new("welcome.finish", Category::Help, finish_welcome).hidden());
    out.push(CommandSpec::new("view.zenMode", Category::View, zen).keys([k("Mod+K Z")]));
    out.push(
        CommandSpec::new("view.exitZenMode", Category::View, exit_zen)
            .keys([k("Escape Escape").when("zenMode")])
            .when("zenMode")
            .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "editor.changeIndentation",
            Category::View,
            change_indentation,
        )
        .when("hasActiveEditor")
        .expect("valid"),
    );
    out.push(
        CommandSpec::new("editor.changeTabSize", Category::View, change_tab_size)
            .when("hasActiveEditor")
            .expect("valid")
            .hidden(),
    );
}

fn category_from(s: &str) -> Option<SettingCategory> {
    crate::settings_tab::CATEGORIES
        .iter()
        .copied()
        .find(|c| crate::settings_tab::category_key(*c).ends_with(&format!(".{s}")))
}

/// `{ category? }`.
fn settings(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let cat = arg(inv, "category").and_then(category_from);
    core.open_settings(cat);
    CommandOutcome::Done
}

fn shortcuts(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.open_settings(Some(SettingCategory::Keyboard));
    CommandOutcome::Done
}

fn welcome(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.open_welcome();
    CommandOutcome::Done
}

fn finish_welcome(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.finish_welcome();
    CommandOutcome::Done
}

fn zen(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let full = inv
        .args
        .get("fullScreen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    core.toggle_zen(full);
    CommandOutcome::Done
}

fn exit_zen(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_zen();
    CommandOutcome::Done
}

/// No args: the indentation picker. `{ action: spaces | tabs | detect |
/// toSpaces | toTabs | tabSize }`.
fn change_indentation(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(doc) = core.workbench.active_document() else {
        return CommandOutcome::Done;
    };
    let Some(action) = arg(inv, "action") else {
        core.effects
            .push(Effect::OpenPicker(PickerKind::Indentation));
        return CommandOutcome::Done;
    };
    let current = core.indent_style(doc);
    match action {
        "spaces" | "tabs" => {
            let style = IndentStyle {
                insert_spaces: action == "spaces",
                ..current
            };
            if let Some(d) = core.workbench.document_mut(doc) {
                d.indent_override = Some(style);
            }
        }
        "detect" => {
            if let Some(d) = core.workbench.document_mut(doc) {
                d.indent_override = None;
                d.indentation = notoir_editor::indent_detect::detect(&d.buffer.text());
            }
        }
        "toSpaces" | "toTabs" => {
            let to_spaces = action == "toSpaces";
            let Some(view) = core.workbench.active_view() else {
                return CommandOutcome::Done;
            };
            let style = IndentStyle {
                insert_spaces: to_spaces,
                ..current
            };
            if let Some(r) = core
                .workbench
                .view(view)
                .and_then(|v| core.workbench.document(v.document_id).map(|d| (d, v)))
                .and_then(|(d, v)| {
                    notoir_editor::line_ops::convert_indentation(
                        d.buffer.rope(),
                        &v.selection,
                        style,
                    )
                })
            {
                let now = core.now_ms;
                let _ = core
                    .workbench
                    .edit(view, r.changes, r.selection, r.kind, now);
            }
            if let Some(d) = core.workbench.document_mut(doc) {
                d.indent_override = Some(style);
            }
        }
        "tabSize" => core.effects.push(Effect::OpenPicker(PickerKind::TabSize)),
        _ => return bad_args(inv),
    }
    CommandOutcome::Done
}

/// `{ size: 1..=8 }`.
fn change_tab_size(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(doc) = core.workbench.active_document() else {
        return CommandOutcome::Done;
    };
    match inv.args.get("size").and_then(|v| v.as_u64()) {
        Some(n @ 1..=8) => {
            let current = core.indent_style(doc);
            if let Some(d) = core.workbench.document_mut(doc) {
                d.indent_override = Some(IndentStyle {
                    tab_size: n as usize,
                    ..current
                });
            }
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}
