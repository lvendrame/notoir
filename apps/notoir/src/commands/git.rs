//! Git read commands (docs/specs/28): Compare with HEAD / Revision, File
//! History, Open File at Revision, Refresh.

use std::path::PathBuf;

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::ViewId;

use super::{Specs, arg, bad_args};
use crate::compare::CompareSource;
use crate::core::AppCore;
use crate::git::{BlobPurpose, HistoryMode};

pub(super) fn specs(out: &mut Specs) {
    let spec = |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, Category::Git, f)
            .when("gitRepo")
            .expect("valid")
    };
    out.push(spec("git.compareWithHead", compare_with_head).icon("git-compare"));
    out.push(spec("git.compareWithRevision", |c, i| {
        history(c, i, HistoryMode::CompareCurrent)
    }));
    out.push(spec("git.fileHistory", |c, i| {
        history(c, i, HistoryMode::Changes)
    }));
    out.push(spec("git.openFileAtRevision", |c, i| {
        history(c, i, HistoryMode::OpenAtRevision)
    }));
    out.push(spec("git.refresh", |c, _| {
        c.git_refresh_soon();
        CommandOutcome::Done
    }));
    out.push(CommandSpec::new("git.historyPick", Category::Git, history_pick).hidden());
}

/// `{ "path" }`, `{ "view" }` or the active editor's file.
fn target_path(core: &AppCore, inv: &CommandInvocation) -> Option<PathBuf> {
    if let Some(p) = arg(inv, "path") {
        return Some(PathBuf::from(p));
    }
    let view = match inv.args.get("view").and_then(|v| v.as_u64()) {
        Some(id) => Some(ViewId(id)),
        None => core.workbench.active_editor(),
    }?;
    let doc = core.workbench.view(view)?.document_id;
    core.workbench.document(doc)?.path().map(PathBuf::from)
}

/// The current side for `path`: its open document, else the file.
fn current(core: &AppCore, path: &std::path::Path) -> CompareSource {
    let (_, key) = notoir_workspace::fs_io::identity(path);
    match core.workbench.find_by_key(&key) {
        Some(doc) => CompareSource::Document(doc),
        None => CompareSource::File(path.to_path_buf()),
    }
}

fn compare_with_head(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(path) = target_path(core, inv) else {
        return bad_args(inv);
    };
    let right = current(core, &path);
    core.compare(CompareSource::GitHead(path), right, false);
    CommandOutcome::Done
}

fn history(core: &mut AppCore, inv: &CommandInvocation, mode: HistoryMode) -> CommandOutcome {
    match target_path(core, inv) {
        Some(path) => {
            core.git_history(path, mode);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// A commit chosen in the File History picker.
fn history_pick(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let (Some(rev), Some(h)) = (arg(inv, "rev"), core.git.history.clone()) else {
        return bad_args(inv);
    };
    let rev = rev.to_owned();
    match h.mode {
        HistoryMode::Changes => core.compare(
            CompareSource::GitRevision(h.path.clone(), format!("{rev}~1")),
            CompareSource::GitRevision(h.path, rev),
            false,
        ),
        HistoryMode::CompareCurrent => {
            let right = current(core, &h.path);
            core.compare(CompareSource::GitRevision(h.path, rev), right, false);
        }
        HistoryMode::OpenAtRevision => {
            core.git_read_blob(BlobPurpose::Open { path: h.path, rev });
        }
    }
    CommandOutcome::Done
}
