//! Recovery, external-change banners and autosave commands (docs/specs/09,
//! 10). Banner commands take `{ "doc": id }`.

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::DocumentId;
use notoir_core::settings::SettingValue;

use super::{Specs, arg, bad_args};
use crate::core::AppCore;
use crate::files::Prompt;

pub(super) fn specs(out: &mut Specs) {
    out.push(CommandSpec::new("file.reloadFromDisk", Category::File, reload).hidden());
    out.push(CommandSpec::new("file.keepMine", Category::File, keep_mine).hidden());
    out.push(CommandSpec::new("file.useDiskVersion", Category::File, use_disk).hidden());
    out.push(CommandSpec::new("file.autoSave", Category::File, auto_save));
    out.push(CommandSpec::new("recovery.show", Category::Help, show));
    out.push(CommandSpec::new("recovery.restoreAll", Category::Help, restore_all).hidden());
    out.push(CommandSpec::new("recovery.review", Category::Help, review).hidden());
    out.push(CommandSpec::new("recovery.discard", Category::Help, discard).hidden());
    out.push(CommandSpec::new("recovery.restoreEntry", Category::Help, restore_entry).hidden());
    out.push(CommandSpec::new("recovery.discardEntry", Category::Help, discard_entry).hidden());
    out.push(CommandSpec::new("recovery.flush", Category::Help, flush).hidden());
    out.push(CommandSpec::new("editor.type", Category::Edit, type_text).hidden());
}

fn doc_arg(core: &AppCore, inv: &CommandInvocation) -> Option<DocumentId> {
    match inv.args.get("doc").and_then(|v| v.as_u64()) {
        Some(id) => core.workbench.document(DocumentId(id)).map(|d| d.id),
        None => core.workbench.active_document(),
    }
}

fn reload(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match doc_arg(core, inv) {
        Some(doc) => {
            core.reload_from_disk(doc, true);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn keep_mine(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match doc_arg(core, inv) {
        Some(doc) => {
            core.keep_my_version(doc);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn use_disk(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match doc_arg(core, inv) {
        Some(doc) => {
            core.reload_from_disk(doc, false);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// `{ mode: off | afterDelay | onFocusChange | onWindowFocusLoss }`; without
/// a mode, toggles between off and afterDelay.
fn auto_save(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let mode = match arg(inv, "mode") {
        Some(m @ ("off" | "afterDelay" | "onFocusChange" | "onWindowFocusLoss")) => m.to_owned(),
        Some(_) => return bad_args(inv),
        None if core.settings.str("files.autoSave", None) == "off" => "afterDelay".into(),
        None => "off".into(),
    };
    core.set_user_setting("files.autoSave", Some(SettingValue::Str(mode)));
    CommandOutcome::Done
}

/// Help → Recover Unsaved Documents…: looks for left-over sessions again.
fn show(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.scan_orphans();
    // Hot-exit sessions of other workspaces are offered here too.
    let other = std::mem::take(&mut core.recovery.other_hot);
    core.recovery.prompt.extend(other);
    if core.recovery.prompt.is_empty() {
        let now = core.now_ms;
        core.notifications.notify(
            notoir_core::notifications::Notification::new(
                notoir_core::notifications::Severity::Info,
                notoir_core::notifications::Translatable::new("notifications:recoveryNothing"),
            ),
            now,
        );
    }
    CommandOutcome::Done
}

fn restore_all(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.recovery_restore_all();
    CommandOutcome::Done
}

fn review(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.recovery.review = !core.recovery.review;
    CommandOutcome::Done
}

fn discard(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let count = core.recovery.prompt_entries().len();
    core.files
        .prompts
        .push_back(Prompt::DiscardRecovery { count });
    CommandOutcome::Done
}

fn restore_entry(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "id") {
        Some(id) => {
            let id = id.to_owned();
            core.recovery_entry(&id, true);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn discard_entry(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "id") {
        Some(id) => {
            let id = id.to_owned();
            core.recovery_entry(&id, false);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// Writes recovery snapshots now (tests and the crash suite).
fn flush(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.flush_recovery();
    CommandOutcome::Done
}

/// `{ text }`: types into the active editor at its selection (test channel,
/// automation).
fn type_text(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let (Some(text), Some(view)) = (arg(inv, "text"), core.workbench.active_view()) else {
        return bad_args(inv);
    };
    let Some(sel) = core.workbench.view(view).map(|v| v.selection.clone()) else {
        return bad_args(inv);
    };
    if let Some(r) = notoir_editor::edit_ops::insert(&sel, text, notoir_editor::EditKind::Other) {
        let now = core.now_ms;
        let _ = core
            .workbench
            .edit(view, r.changes, r.selection, r.kind, now);
    }
    CommandOutcome::Done
}
