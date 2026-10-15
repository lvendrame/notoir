//! Folding, bookmarks, Go to Symbol, Outline and breadcrumbs commands
//! (docs/specs/12, 26).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::ViewId;
use notoir_core::keymap::KeyBindingDef;
use notoir_core::panels::PanelId;
use notoir_core::settings::SettingValue;

use super::Specs;
use crate::core::{AppCore, Effect};
use crate::shell::overlays::PickerKind;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    let editor = "hasActiveEditor";
    let spec = |id: &'static str,
                cat: Category,
                f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, cat, f).when(editor).expect("valid")
    };
    out.push(
        spec("fold.fold", Category::View, |c, i| fold(c, i, "fold"))
            .keys([k("Mod+Shift+[").mac("Mod+Alt+[")]),
    );
    out.push(
        spec("fold.unfold", Category::View, |c, i| fold(c, i, "unfold"))
            .keys([k("Mod+Shift+]").mac("Mod+Alt+]")]),
    );
    out.push(
        spec("fold.toggle", Category::View, |c, i| fold(c, i, "toggle")).keys([k("Mod+K Mod+L")]),
    );
    out.push(
        spec("fold.foldAll", Category::View, |c, _| {
            with_view(c, |c, v| c.fold_all(v, None, false))
        })
        .keys([k("Mod+K Mod+0")]),
    );
    out.push(
        spec("fold.unfoldAll", Category::View, |c, _| {
            with_view(c, |c, v| c.unfold_all(v))
        })
        .keys([k("Mod+K Mod+J")]),
    );
    out.push(spec("fold.foldAllExceptCurrent", Category::View, |c, _| {
        with_view(c, |c, v| c.fold_all(v, None, true))
    }));
    macro_rules! level {
        ($($n:literal),*) => {$(
            out.push(
                spec(concat!("fold.foldLevel", $n), Category::View, |c, _| with_view(c, |c, v| c.fold_all(v, Some($n), false)))
                    .keys([k(concat!("Mod+K Mod+", $n))]),
            );
        )*};
    }
    level!(1, 2, 3, 4, 5, 6, 7);
    out.push(
        spec("bookmarks.toggle", Category::Navigate, toggle_bookmark)
            .keys([k("Mod+F2")])
            .icon("bookmark"),
    );
    out.push(
        spec("bookmarks.next", Category::Navigate, |c, _| {
            with_view(c, |c, v| c.next_bookmark(v, true))
        })
        .keys([k("F2").when("editorFocus")]),
    );
    out.push(
        spec("bookmarks.previous", Category::Navigate, |c, _| {
            with_view(c, |c, v| c.next_bookmark(v, false))
        })
        .keys([k("Shift+F2").when("editorFocus")]),
    );
    out.push(spec("bookmarks.clear", Category::Navigate, |c, _| {
        let doc = c.workbench.active_document();
        if doc.is_some() {
            c.clear_bookmarks(doc);
        }
        CommandOutcome::Done
    }));
    out.push(CommandSpec::new(
        "bookmarks.clearAll",
        Category::Navigate,
        |c, _| {
            c.clear_bookmarks(None);
            CommandOutcome::Done
        },
    ));
    out.push(
        spec("navigate.goToSymbol", Category::Navigate, go_to_symbol).keys([k("Mod+Shift+O")]),
    );
    out.push(CommandSpec::new("navigate.revealRange", Category::Navigate, reveal_range).hidden());
    out.push(CommandSpec::new(
        "view.showOutline",
        Category::View,
        show_outline,
    ));
    out.push(CommandSpec::new(
        "view.toggleBreadcrumbs",
        Category::View,
        toggle_breadcrumbs,
    ));
    out.push(
        spec("breadcrumbs.focus", Category::Navigate, focus_breadcrumbs).keys([k("Mod+Shift+.")]),
    );
}

fn with_view(core: &mut AppCore, f: impl FnOnce(&mut AppCore, ViewId)) -> CommandOutcome {
    if let Some(v) = core.workbench.active_view() {
        f(core, v);
    }
    CommandOutcome::Done
}

fn line_arg(inv: &CommandInvocation) -> Option<usize> {
    inv.args
        .get("line")
        .and_then(|v| v.as_u64())
        .map(|l| l as usize)
}

/// `{ line? }` (0-based; the gutter passes it, the keyboard uses the caret).
fn fold(core: &mut AppCore, inv: &CommandInvocation, op: &str) -> CommandOutcome {
    let line = line_arg(inv);
    with_view(core, |c, v| c.fold_op(v, line, op))
}

fn toggle_bookmark(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let line = line_arg(inv);
    with_view(core, |c, v| c.toggle_bookmark(v, line))
}

fn go_to_symbol(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::OpenPicker(PickerKind::Symbols));
    CommandOutcome::Done
}

/// `{ start, end, view? }`: select and centre (outline, breadcrumbs, symbols).
fn reveal_range(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let start = inv.args.get("start").and_then(|v| v.as_u64());
    let end = inv.args.get("end").and_then(|v| v.as_u64());
    let view = inv
        .args
        .get("view")
        .and_then(|v| v.as_u64())
        .map(ViewId)
        .or(core.workbench.active_view());
    match (start, end, view) {
        (Some(a), Some(b), Some(v)) => {
            if core.workbench.active_view() != Some(v) {
                let _ = core.workbench.activate_view(v);
            }
            core.reveal_range(v, a as usize, b as usize);
            CommandOutcome::Done
        }
        _ => super::bad_args(inv),
    }
}

fn show_outline(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.panels.show(PanelId::Outline);
    CommandOutcome::Done
}

fn toggle_breadcrumbs(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let on = core.settings.bool("workbench.breadcrumbs.enabled", None);
    core.set_user_setting(
        "workbench.breadcrumbs.enabled",
        Some(SettingValue::Bool(!on)),
    );
    CommandOutcome::Done
}

fn focus_breadcrumbs(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.breadcrumbs_focus = true;
    CommandOutcome::Done
}
