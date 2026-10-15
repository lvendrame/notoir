//! Workspace search, Quick Open and navigation history commands
//! (docs/specs/15).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::keymap::KeyBindingDef;
use notoir_core::panels::PanelId;

use super::{Specs, bad_args};
use crate::core::{AppCore, Effect};
use crate::shell::overlays::PickerKind;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    out.push(
        CommandSpec::new("search.workspace", Category::Edit, search_workspace)
            .keys([k("Mod+Shift+F")])
            .icon("search"),
    );
    out.push(CommandSpec::new(
        "search.replaceAll",
        Category::Edit,
        |c: &mut AppCore, _: &CommandInvocation| {
            c.request_replace_all();
            CommandOutcome::Done
        },
    ));
    out.push(
        CommandSpec::new(
            "search.next",
            Category::Navigate,
            |c: &mut AppCore, _: &CommandInvocation| step(c, true),
        )
        .keys([k("F4")]),
    );
    out.push(
        CommandSpec::new(
            "search.previous",
            Category::Navigate,
            |c: &mut AppCore, _: &CommandInvocation| step(c, false),
        )
        .keys([k("Shift+F4")]),
    );
    out.push(CommandSpec::new("search.openResult", Category::Navigate, open_result).hidden());
    out.push(CommandSpec::new("search.dismiss", Category::Edit, dismiss).hidden());
    out.push(CommandSpec::new("search.replaceOne", Category::Edit, replace_one).hidden());
    out.push(CommandSpec::new("search.undoReplace", Category::Edit, undo_replace).hidden());
    out.push(CommandSpec::new("quickOpen.open", Category::Navigate, quick_open_target).hidden());
    out.push(CommandSpec::new("search.copyPath", Category::File, copy_path).hidden());
    out.push(
        CommandSpec::new("explorer.findInFolder", Category::File, find_in_folder)
            .when("hasWorkspace")
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("navigate.quickOpen", Category::Navigate, quick_open).keys([k("Mod+P")]),
    );
    out.push(
        CommandSpec::new(
            "navigate.back",
            Category::Navigate,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.nav_step(false);
                CommandOutcome::Done
            },
        )
        .keys([k("Alt+Left").mac("Ctrl+-")]),
    );
    out.push(
        CommandSpec::new(
            "navigate.forward",
            Category::Navigate,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.nav_step(true);
                CommandOutcome::Done
            },
        )
        .keys([k("Alt+Right").mac("Ctrl+Shift+-")]),
    );
}

/// Shows the Search panel and focuses the query, seeded with a one-line
/// selection.
fn search_workspace(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.panels.show(PanelId::Search);
    if let Some(v) = core
        .workbench
        .active_view()
        .and_then(|v| core.workbench.view(v))
        && !v.selection.primary().is_empty()
        && let Some(d) = core.workbench.document(v.document_id)
    {
        let r = v.selection.primary();
        let text: String = d.buffer.rope().slice(r.from()..r.to()).to_string();
        if !text.contains('\n') {
            core.search.query = text;
            core.search.due = Some(0);
        }
    }
    core.search.focus_query = true;
    core.editor_focused = false;
    CommandOutcome::Done
}

fn step(core: &mut AppCore, forward: bool) -> CommandOutcome {
    // The find widget's matches when it was used last, else the panel's.
    if !core.search_step(forward) {
        let id = if forward {
            "edit.findNext"
        } else {
            "edit.findPrevious"
        };
        core.enqueue(CommandInvocation::new(
            id,
            notoir_core::command::InvocationSource::Internal,
        ));
    }
    CommandOutcome::Done
}

fn idx(inv: &CommandInvocation, key: &str) -> Option<usize> {
    inv.args
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
}

/// `{ file, match, side? }`.
fn open_result(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let side = inv
        .args
        .get("side")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match (idx(inv, "file"), idx(inv, "match")) {
        (Some(f), Some(m)) => {
            core.open_search_result(f, m, side);
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

/// `{ file, match? }`.
fn dismiss(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match idx(inv, "file") {
        Some(f) => {
            core.dismiss_result(f, idx(inv, "match"));
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// `{ file, match? }`: replace in one file or one match.
fn replace_one(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match idx(inv, "file") {
        Some(f) => {
            core.replace_results(Some((f, idx(inv, "match"))));
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn copy_path(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match super::arg(inv, "path") {
        Some(p) => {
            core.effects.push(Effect::CopyText(p.to_owned()));
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// Explorer → Find in Folder…: `files to include` = the folder.
fn find_in_folder(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(path) = core.explorer_target(inv) else {
        return bad_args(inv);
    };
    let rel = core
        .workspace
        .as_ref()
        .and_then(|w| w.relative(&path))
        .unwrap_or_default();
    core.search.include = if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/**")
    };
    core.panels.show(PanelId::Search);
    core.search.focus_query = true;
    core.search_changed();
    CommandOutcome::Done
}

fn undo_replace(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.undo_replace();
    CommandOutcome::Done
}

/// `{ path | doc, line?, col?, side? }` (1-based line/col).
fn quick_open_target(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let line = idx(inv, "line");
    let col = idx(inv, "col").unwrap_or(1);
    let side = inv
        .args
        .get("side")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let at = crate::files::OpenAt {
        line_col: line.map(|l| (l, col)),
        side,
        ..Default::default()
    };
    if let Some(p) = super::arg(inv, "path") {
        core.open_path(std::path::Path::new(p), at);
        return CommandOutcome::Done;
    }
    let Some(doc) = idx(inv, "doc").map(|d| notoir_core::ids::DocumentId(d as u64)) else {
        return bad_args(inv);
    };
    let group = if side {
        core.workbench.side_group()
    } else {
        core.workbench.layout.active_group
    };
    let view = core.workbench.view_in_group(doc, group).or_else(|| {
        core.workbench
            .open_view(doc, group, notoir_core::TabPosition::AfterActive)
            .ok()
    });
    if let Some(v) = view {
        let _ = core.workbench.activate_view(v);
        if let Some((l, c)) = at.line_col
            && let Some(rope) = core
                .workbench
                .document(doc)
                .map(|d| d.buffer.rope().clone())
        {
            let l = (l - 1).min(rope.len_lines().saturating_sub(1));
            let pos = (rope.line_to_char(l) + c - 1).min(rope.len_chars());
            let _ = core
                .workbench
                .set_selection(v, notoir_editor::Selection::caret(pos));
            core.find.center.push(v);
        }
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}

fn quick_open(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::OpenPicker(PickerKind::QuickOpen));
    CommandOutcome::Done
}
