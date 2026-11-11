//! Theme and display-language commands (docs/specs/21, 22).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::settings::SettingValue;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect, ThemeChoice};
use crate::i18n::Language;

pub(super) fn specs(out: &mut Specs) {
    out.push(
        CommandSpec::new(
            "preferences.manageThemes",
            Category::Preferences,
            manage_themes,
        )
        .icon("palette"),
    );
    out.push(CommandSpec::new("themes.action", Category::Preferences, theme_action).hidden());
    out.push(
        CommandSpec::new(
            "preferences.changeTheme",
            Category::Preferences,
            change_theme,
        )
        .icon("palette"),
    );
    out.push(
        CommandSpec::new(
            "preferences.changeLanguage",
            Category::Preferences,
            change_language,
        )
        .icon("globe"),
    );
}

/// No args: open the theme picker. `{ "theme": "<id>|auto" }`: apply and persist.
fn change_theme(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(value) = arg(inv, "theme") else {
        core.effects.push(Effect::OpenPicker(
            crate::shell::overlays::PickerKind::Theme,
        ));
        return CommandOutcome::Done;
    };
    let Some(choice) = ThemeChoice::from_setting(value) else {
        return bad_args(inv);
    };
    let value = choice.setting_value();
    core.theme_choice = choice;
    core.theme_override = None;
    core.set_user_setting("workbench.theme", Some(SettingValue::Str(value)));
    core.effects.push(Effect::ApplyTheme);
    CommandOutcome::Done
}

/// No args: open the language picker. `{ "language": "<code>" }`: apply and persist.
fn change_language(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(code) = arg(inv, "language") else {
        core.effects.push(Effect::OpenPicker(
            crate::shell::overlays::PickerKind::Language,
        ));
        return CommandOutcome::Done;
    };
    let Some(language) = Language::from_code(code) else {
        return bad_args(inv);
    };
    core.language = language;
    core.set_user_setting(
        "language",
        Some(SettingValue::Str(language.code().to_owned())),
    );
    core.effects.push(Effect::ApplyLanguage);
    CommandOutcome::Done
}

fn manage_themes(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.open_virtual("tabs:themes", notoir_core::view::ViewKind::ThemeManager);
    if core.theme_manager.draft.is_none() {
        let id = core.theme(core.appearance_dark);
        core.select_theme_draft(&id);
    }
    core.request_theme_fonts();
    CommandOutcome::Done
}

fn theme_action(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    core.theme_action(&inv.args);
    CommandOutcome::Done
}
