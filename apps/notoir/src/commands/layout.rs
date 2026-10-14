//! Editor layout commands: splits, single editor, maximize, group focus and
//! moving tabs between groups (docs/specs/03).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::{GroupId, ViewId};
use notoir_core::keymap::KeyBindingDef;
use notoir_core::layout::Direction;
use notoir_core::workbench::TabPosition;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect};

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    out.push(CommandSpec::new(
        "view.splitLeft",
        Category::View,
        split_left,
    ));
    out.push(CommandSpec::new("view.splitRight", Category::View, split_right).keys([k("Mod+\\")]));
    out.push(CommandSpec::new("view.splitUp", Category::View, split_up));
    out.push(CommandSpec::new(
        "view.splitDown",
        Category::View,
        split_down,
    ));
    out.push(CommandSpec::new(
        "view.singleEditor",
        Category::View,
        single_editor,
    ));
    out.push(
        CommandSpec::new("workbench.maximizeGroup", Category::View, maximize).icon("maximize"),
    );
    out.push(
        CommandSpec::new("workbench.focusNextGroup", Category::Navigate, focus_next)
            .keys([k("Mod+K Mod+Right")]),
    );
    out.push(
        CommandSpec::new(
            "workbench.focusPreviousGroup",
            Category::Navigate,
            focus_previous,
        )
        .keys([k("Mod+K Mod+Left")]),
    );
    out.push(CommandSpec::new("workbench.focusGroup", Category::Navigate, focus_group).hidden());
    out.push(
        CommandSpec::new("workbench.moveTabToNextGroup", Category::View, move_next)
            .when("hasActiveEditor")
            .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "workbench.moveTabToPreviousGroup",
            Category::View,
            move_previous,
        )
        .when("hasActiveEditor")
        .expect("valid"),
    );
    out.push(CommandSpec::new("view.moveTab", Category::View, move_tab).hidden());
    out.push(CommandSpec::new("view.dockTab", Category::View, dock_tab).hidden());
}

fn view_arg(core: &AppCore, inv: &CommandInvocation) -> Option<ViewId> {
    match inv.args.get("view").and_then(|v| v.as_u64()) {
        Some(id) => core.workbench.view(ViewId(id)).map(|v| v.id),
        None => core.workbench.active_view(),
    }
}

fn group_arg(core: &AppCore, inv: &CommandInvocation) -> Option<GroupId> {
    let id = inv.args.get("group").and_then(|v| v.as_u64())?;
    core.workbench.layout.group(GroupId(id)).map(|g| g.id)
}

fn split(core: &mut AppCore, inv: &CommandInvocation, dir: Direction) -> CommandOutcome {
    core.exit_maximize();
    if let Some(v) = view_arg(core, inv) {
        let _ = core.workbench.activate_view(v);
    }
    core.workbench.split_active(dir);
    if let Some(v) = core.workbench.active_view() {
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}

fn split_left(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    split(core, inv, Direction::Left)
}

fn split_right(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    split(core, inv, Direction::Right)
}

fn split_up(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    split(core, inv, Direction::Up)
}

fn split_down(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    split(core, inv, Direction::Down)
}

fn single_editor(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.exit_maximize();
    core.workbench.single_editor();
    CommandOutcome::Done
}

fn maximize(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.toggle_maximize();
    CommandOutcome::Done
}

fn focus(core: &mut AppCore, group: GroupId) {
    core.exit_maximize();
    core.workbench.layout.active_group = group;
    if let Some(v) = core.workbench.layout.group(group).and_then(|g| g.active) {
        let _ = core.workbench.activate_view(v);
        core.effects.push(Effect::FocusEditor(v));
    }
}

fn focus_next(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let g = core
        .workbench
        .layout
        .neighbor(core.workbench.layout.active_group, true);
    focus(core, g);
    CommandOutcome::Done
}

fn focus_previous(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let g = core
        .workbench
        .layout
        .neighbor(core.workbench.layout.active_group, false);
    focus(core, g);
    CommandOutcome::Done
}

fn focus_group(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match group_arg(core, inv) {
        Some(g) => {
            core.workbench.layout.active_group = g;
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn move_neighbor(core: &mut AppCore, inv: &CommandInvocation, forward: bool) -> CommandOutcome {
    core.exit_maximize();
    if let Some(v) = view_arg(core, inv)
        && core.workbench.move_view_to_neighbor(v, forward).is_ok()
    {
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}

fn move_next(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    move_neighbor(core, inv, true)
}

fn move_previous(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    move_neighbor(core, inv, false)
}

/// `{ view, group, index?, copy }`: move (or copy) a tab into a strip.
fn move_tab(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let (Some(view), Some(group)) = (view_arg(core, inv), group_arg(core, inv)) else {
        return bad_args(inv);
    };
    let copy = inv
        .args
        .get("copy")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let len = core
        .workbench
        .layout
        .group(group)
        .map_or(0, |g| g.tabs.len());
    let index = inv
        .args
        .get("index")
        .and_then(|v| v.as_u64())
        .map_or(len, |i| i as usize);
    let result = if copy {
        core.workbench
            .duplicate_into(view, group, TabPosition::Index(index))
            .map(Some)
    } else {
        core.workbench
            .move_view(view, group, index)
            .map(|()| Some(view))
    };
    if let Ok(Some(v)) = result {
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}

/// `{ view, group, dir: left|right|up|down, copy }`: dock into a new split.
fn dock_tab(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let (Some(view), Some(group)) = (view_arg(core, inv), group_arg(core, inv)) else {
        return bad_args(inv);
    };
    let dir = match arg(inv, "dir") {
        Some("left") => Direction::Left,
        Some("right") => Direction::Right,
        Some("up") => Direction::Up,
        Some("down") => Direction::Down,
        _ => return bad_args(inv),
    };
    let copy = inv
        .args
        .get("copy")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let result = if copy {
        core.workbench.duplicate_to_split(view, group, dir)
    } else {
        core.workbench.move_view_to_split(view, group, dir)
    };
    if result.is_ok()
        && let Some(v) = core.workbench.active_view()
    {
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}
