//! View commands: panels, sidebars, toolbar, status bar, full screen
//! (docs/specs/17, 18).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::KeyBindingDef;
use notoir_core::panels::{PanelArea, PanelId, Side};
use notoir_core::settings::SettingValue;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect, panel_from_key};

pub(super) fn specs(out: &mut Specs) {
    out.push(
        CommandSpec::new("view.togglePrimarySidebar", Category::View, toggle_primary)
            .keys([
                KeyBindingDef::new("Mod+B").when("!(editorFocus && languageId == 'markdown')"),
                KeyBindingDef::new("Mod+K B"),
            ])
            .icon("panel-left"),
    );
    out.push(
        CommandSpec::new(
            "view.toggleSecondarySidebar",
            Category::View,
            toggle_secondary,
        )
        .keys([KeyBindingDef::new("Mod+Alt+B")])
        .icon("panel-right"),
    );
    out.push(
        CommandSpec::new("view.toggleBottomPanel", Category::View, toggle_bottom)
            .keys([KeyBindingDef::new("Mod+J")])
            .icon("panel-bottom"),
    );
    out.push(CommandSpec::new(
        "view.toggleToolbar",
        Category::View,
        toggle_toolbar,
    ));
    out.push(CommandSpec::new(
        "view.toggleStatusBar",
        Category::View,
        toggle_status_bar,
    ));
    out.push(CommandSpec::new(
        "view.toggleActivityBar",
        Category::View,
        toggle_activity_bar,
    ));
    out.push(CommandSpec::new(
        "view.togglePrimarySidebarPosition",
        Category::View,
        toggle_primary_side,
    ));
    out.push(
        CommandSpec::new("view.showExplorer", Category::View, show_explorer)
            .keys([KeyBindingDef::new("Mod+Shift+E")]),
    );
    out.push(
        CommandSpec::new("view.showOutput", Category::View, show_output)
            .keys([KeyBindingDef::new("Mod+Shift+U")]),
    );
    out.push(CommandSpec::new("view.togglePanel", Category::View, toggle_panel).hidden());
    out.push(CommandSpec::new("view.movePanel", Category::View, move_panel).hidden());
    out.push(
        CommandSpec::new("view.toggleFullScreen", Category::View, toggle_full_screen)
            .keys([KeyBindingDef::new("F11").mac("Ctrl+Cmd+F")]),
    );
}

fn toggle_primary(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    core.panels.toggle_area(PanelArea::Primary);
    CommandOutcome::Done
}

fn toggle_secondary(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    core.panels.toggle_area(PanelArea::Secondary);
    CommandOutcome::Done
}

fn toggle_bottom(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    core.panels.toggle_area(PanelArea::Bottom);
    CommandOutcome::Done
}

fn toggle_bool_setting(core: &mut AppCore, key: &str) -> CommandOutcome {
    let current = core.settings.bool(key, None);
    core.set_user_setting(key, Some(SettingValue::Bool(!current)));
    CommandOutcome::Done
}

fn toggle_toolbar(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_bool_setting(core, "workbench.toolbar.visible")
}

fn toggle_status_bar(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_bool_setting(core, "workbench.statusBar.visible")
}

fn toggle_activity_bar(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.panels.activity_bar_visible = !core.panels.activity_bar_visible;
    CommandOutcome::Done
}

fn toggle_primary_side(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let side = match core.panels.primary_side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    };
    core.panels.set_primary_side(side);
    CommandOutcome::Done
}

fn show_explorer(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    core.panels.show(PanelId::Explorer);
    CommandOutcome::Done
}

fn show_output(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    core.panels.show(PanelId::Output);
    CommandOutcome::Done
}

/// Args: `{ "panel": "explorer" }`.
fn toggle_panel(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    match arg(inv, "panel").and_then(panel_from_key) {
        Some(p) => {
            core.panels.toggle(p);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// Args: `{ "panel": "outline", "area": "primary" | "secondary" | "bottom" }`.
fn move_panel(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    let area = match arg(inv, "area") {
        Some("primary") => PanelArea::Primary,
        Some("secondary") => PanelArea::Secondary,
        Some("bottom") => PanelArea::Bottom,
        _ => return bad_args(inv),
    };
    match arg(inv, "panel").and_then(panel_from_key) {
        Some(p) if core.panels.move_panel(p, area) => CommandOutcome::Done,
        _ => bad_args(inv),
    }
}

fn toggle_full_screen(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::ToggleFullScreen);
    CommandOutcome::Done
}
