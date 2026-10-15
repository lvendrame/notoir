//! Compare commands (docs/specs/27): pick sources, Select for Compare,
//! Compare Current Changes, change navigation, hunk copy and diff options.

use std::path::PathBuf;

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::document::DocumentOrigin;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::keymap::KeyBindingDef;
use notoir_editor::diff::Side;

use super::{Specs, arg, bad_args};
use crate::compare::CompareSource;
use crate::core::{AppCore, Effect};
use crate::shell::overlays::PickerKind;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    let in_diff = |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, Category::Compare, f)
            .when("inDiff")
            .expect("valid")
    };
    out.push(CommandSpec::new("compare.open", Category::Compare, open).icon("git-compare"));
    out.push(CommandSpec::new("compare.with", Category::Compare, with));
    out.push(CommandSpec::new(
        "compare.select",
        Category::Compare,
        select,
    ));
    out.push(
        CommandSpec::new("compare.withSelected", Category::Compare, with_selected)
            .when("compareSelected")
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("compare.currentChanges", Category::Compare, current_changes)
            .when("hasActiveEditor")
            .expect("valid"),
    );
    out.push(in_diff("compare.nextChange", |c, i| step(c, i, true)).keys([k("F7")]));
    out.push(in_diff("compare.previousChange", |c, i| step(c, i, false)).keys([k("Shift+F7")]));
    out.push(in_diff("compare.toggleIgnoreWhitespace", |c, i| {
        toggle(c, i, |o| o.ignore_whitespace = !o.ignore_whitespace)
    }));
    out.push(in_diff("compare.toggleIgnoreCase", |c, i| {
        toggle(c, i, |o| o.ignore_case = !o.ignore_case)
    }));
    out.push(in_diff("compare.swapSides", swap));
    out.push(in_diff("compare.copyChangeToRight", |c, i| {
        copy_current(c, i, Side::Right)
    }));
    out.push(in_diff("compare.copyChangeToLeft", |c, i| {
        copy_current(c, i, Side::Left)
    }));
    out.push(CommandSpec::new("compare.copyHunk", Category::Compare, copy_hunk).hidden());
    out.push(CommandSpec::new("compare.revealHunk", Category::Compare, reveal_hunk).hidden());
    out.push(CommandSpec::new("compare.pick", Category::Compare, pick).hidden());
}

/// A diff tab from `{ "view": id }` or the active tab.
fn diff_tab(core: &AppCore, inv: &CommandInvocation) -> Option<ViewId> {
    match inv.args.get("view").and_then(|v| v.as_u64()) {
        Some(id) => core
            .compare
            .sessions
            .contains_key(&ViewId(id))
            .then_some(ViewId(id)),
        None => core.active_diff(),
    }
}

/// The source a context menu or the active editor names: `{ "path" }`,
/// `{ "view" }`, or the active editor's document.
fn source_of(core: &AppCore, inv: &CommandInvocation) -> Option<CompareSource> {
    if let Some(p) = arg(inv, "path") {
        return Some(CompareSource::File(PathBuf::from(p)));
    }
    let view = match inv.args.get("view").and_then(|v| v.as_u64()) {
        Some(id) => Some(ViewId(id)),
        None => core.workbench.active_editor(),
    }?;
    let doc = core.workbench.view(view)?.document_id;
    doc_source(core, doc)
}

fn doc_source(core: &AppCore, doc: DocumentId) -> Option<CompareSource> {
    let d = core.workbench.document(doc)?;
    // Diff tabs and other virtual pages are not comparable.
    if matches!(d.origin, DocumentOrigin::Virtual { .. }) && core.compare_title(doc).is_none() {
        return None;
    }
    if core.compare.sessions.values().any(|s| s.doc == doc) {
        return None;
    }
    Some(CompareSource::Document(doc))
}

/// Tools → Compare Files…: the active document on the left, then pick the
/// right side (or both when nothing is open).
fn open(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    core.compare.pick_left = source_of(core, inv);
    core.effects.push(Effect::OpenPicker(PickerKind::Compare));
    CommandOutcome::Done
}

/// Compare With…: this tab / file on the left, pick the right side.
fn with(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match source_of(core, inv) {
        Some(src) => {
            core.compare.pick_left = Some(src);
            core.effects.push(Effect::OpenPicker(PickerKind::Compare));
            CommandOutcome::Done
        }
        None => CommandOutcome::Done,
    }
}

fn select(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(src) = source_of(core, inv) {
        core.compare.selected = Some(src);
    }
    CommandOutcome::Done
}

fn with_selected(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let (Some(left), Some(right)) = (core.compare.selected.clone(), source_of(core, inv)) else {
        return CommandOutcome::Done;
    };
    core.compare.selected = None;
    core.compare(left, right, false);
    CommandOutcome::Done
}

fn current_changes(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let doc = match inv.args.get("doc").and_then(|v| v.as_u64()) {
        Some(id) => Some(DocumentId(id)),
        None => core
            .workbench
            .active_editor()
            .and_then(|v| core.workbench.view(v))
            .map(|v| v.document_id),
    };
    match doc.filter(|d| core.workbench.document(*d).is_some()) {
        Some(d) => {
            core.compare_current_changes(d);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn step(core: &mut AppCore, inv: &CommandInvocation, forward: bool) -> CommandOutcome {
    if let Some(tab) = diff_tab(core, inv) {
        core.compare_step(tab, forward);
    }
    CommandOutcome::Done
}

fn toggle(
    core: &mut AppCore,
    inv: &CommandInvocation,
    f: impl FnOnce(&mut notoir_editor::diff::DiffOptions),
) -> CommandOutcome {
    if let Some(tab) = diff_tab(core, inv) {
        core.compare_toggle(tab, f);
    }
    CommandOutcome::Done
}

fn swap(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(tab) = diff_tab(core, inv) {
        core.compare_swap(tab);
    }
    CommandOutcome::Done
}

fn side_arg(inv: &CommandInvocation) -> Option<Side> {
    match arg(inv, "toward")? {
        "left" => Some(Side::Left),
        "right" => Some(Side::Right),
        _ => None,
    }
}

fn copy_hunk(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let hunk = inv.args.get("hunk").and_then(|v| v.as_u64());
    match (diff_tab(core, inv), hunk, side_arg(inv)) {
        (Some(tab), Some(i), Some(toward)) => {
            core.compare_copy_hunk(tab, i as usize, toward);
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

/// Copy Change to Right / Left: the hunk at the focused pane's caret, or the
/// last revealed one.
fn copy_current(core: &mut AppCore, inv: &CommandInvocation, toward: Side) -> CommandOutcome {
    let Some(tab) = diff_tab(core, inv) else {
        return CommandOutcome::Done;
    };
    let focus = core.diff_focus_side(tab);
    let Some(s) = core.compare.sessions.get(&tab) else {
        return CommandOutcome::Done;
    };
    let pane = s.pane(focus);
    let line = core
        .workbench
        .view(pane.view)
        .zip(core.workbench.document(pane.doc))
        .map(|(v, d)| {
            let rope = d.buffer.rope();
            rope.char_to_line(v.selection.primary().head.min(rope.len_chars()))
        });
    let hunk = line.and_then(|l| s.diff.hunk_at(focus, l)).or(s.current);
    if let Some(i) = hunk {
        core.compare_copy_hunk(tab, i, toward);
    }
    CommandOutcome::Done
}

fn reveal_hunk(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let hunk = inv.args.get("hunk").and_then(|v| v.as_u64());
    match (diff_tab(core, inv), hunk) {
        (Some(tab), Some(i)) => {
            core.reveal_hunk(tab, i as usize);
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

/// The Compare picker chose a source (`clipboard`, `p:<path>`, `d:<id>`):
/// the left side first when none was given, then the right side.
fn pick(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(value) = arg(inv, "value") else {
        return bad_args(inv);
    };
    let src = if value == "clipboard" {
        CompareSource::Clipboard
    } else if let Some(p) = value.strip_prefix("p:") {
        CompareSource::File(PathBuf::from(p))
    } else if let Some(d) = value.strip_prefix("d:").and_then(|d| d.parse::<u64>().ok()) {
        CompareSource::Document(DocumentId(d))
    } else {
        return bad_args(inv);
    };
    match core.compare.pick_left.take() {
        Some(left) => core.compare(left, src, false),
        None => {
            core.compare.pick_left = Some(src);
            core.effects.push(Effect::OpenPicker(PickerKind::Compare));
        }
    }
    CommandOutcome::Done
}
