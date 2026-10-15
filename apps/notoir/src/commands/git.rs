//! Git commands (docs/specs/28): Compare with HEAD / Revision, File History,
//! Open File at Revision, Refresh; Source Control (stage, unstage, discard,
//! commit), branches, fetch / pull / push / sync, clone, init, Git Output
//! and Sync Notes.

use std::path::PathBuf;

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::ViewId;
use notoir_core::panels::PanelId;
use notoir_git::PullMode;

use super::{Specs, arg, bad_args};
use crate::compare::CompareSource;
use crate::core::{AppCore, Effect};
use crate::git::{BlobPurpose, HistoryMode};
use crate::git_write::{BranchPick, GitPrompt, LocalOp, NetOp};
use crate::shell::overlays::PickerKind;

type Handler = fn(&mut AppCore, &CommandInvocation) -> CommandOutcome;

pub(super) fn specs(out: &mut Specs) {
    let spec = |id: &'static str, f: Handler| {
        CommandSpec::new(id, Category::Git, f)
            .when("gitRepo")
            .expect("valid")
    };
    // Writes wait while another Git operation runs.
    let write = |id: &'static str, f: Handler| {
        CommandSpec::new(id, Category::Git, f)
            .when("gitRepo && !gitBusy")
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

    // Gutter peek
    out.push(CommandSpec::new("git.peekChange", Category::Git, peek_change).hidden());
    out.push(
        CommandSpec::new(
            "git.revertChange",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.git_revert_change();
                CommandOutcome::Done
            },
        )
        .when("gitPeek")
        .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "git.stageChange",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.git_stage_change();
                CommandOutcome::Done
            },
        )
        .when("gitPeek && gitPeekStageable && !gitBusy")
        .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "git.closePeek",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.git.peek = None;
                CommandOutcome::Done
            },
        )
        .hidden(),
    );

    // Source Control
    out.push(
        CommandSpec::new(
            "git.showSourceControl",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| show(c, PanelId::SourceControl),
        )
        .icon("git-branch"),
    );
    out.push(CommandSpec::new("git.selectRepository", Category::Git, select_repository).hidden());
    out.push(CommandSpec::new(
        "git.showOutput",
        Category::Git,
        |c: &mut AppCore, _: &CommandInvocation| show(c, PanelId::GitOutput),
    ));
    out.push(spec("git.viewFileChanges", view_file_changes));
    out.push(CommandSpec::new("git.openChange", Category::Git, open_change).hidden());
    out.push(CommandSpec::new("git.revealInExplorer", Category::Git, reveal).hidden());
    out.push(write("git.stage", |c, i| paths_op(c, i, LocalOp::Stage)));
    out.push(write("git.stageAll", |c, _| {
        c.git_submit(LocalOp::StageAll);
        CommandOutcome::Done
    }));
    out.push(write("git.unstage", |c, i| {
        paths_op(c, i, LocalOp::Unstage)
    }));
    out.push(write("git.unstageAll", |c, _| {
        let all: Vec<String> = c.git.status.staged.keys().cloned().collect();
        if !all.is_empty() {
            c.git_submit(LocalOp::Unstage(all));
        }
        CommandOutcome::Done
    }));
    out.push(write("git.discard", discard));
    out.push(write("git.discardAll", |c, _| {
        let all: Vec<String> = c.git.status.changes().map(|(k, _)| k.clone()).collect();
        c.git_discard(all);
        CommandOutcome::Done
    }));
    out.push(write("git.commit", commit).icon("check"));
    out.push(write("git.commitAll", |c, i| {
        c.git_commit(message(i), false, true, false);
        CommandOutcome::Done
    }));
    out.push(write("git.commitAmend", |c, i| {
        c.git_commit(message(i), true, false, false);
        CommandOutcome::Done
    }));
    out.push(write("git.commitAndPush", |c, i| {
        c.git_commit(message(i), false, false, true);
        CommandOutcome::Done
    }));

    // Branches
    out.push(write("git.switchBranch", |c, _| {
        c.git_pick_branch(BranchPick::Switch);
        CommandOutcome::Done
    }));
    out.push(write("git.createBranch", create_branch));
    out.push(write("git.createBranchFrom", |c, _| {
        c.git_pick_branch(BranchPick::CreateFrom);
        CommandOutcome::Done
    }));
    out.push(CommandSpec::new("git.branchPick", Category::Git, branch_pick).hidden());
    out.push(CommandSpec::new("git.checkout", Category::Git, checkout).hidden());

    // Network
    out.push(write("git.fetch", |c, _| {
        c.git_start_net(NetOp::Fetch);
        CommandOutcome::Done
    }));
    out.push(write("git.pull", |c, _| {
        c.git_pull(None);
        CommandOutcome::Done
    }));
    out.push(write("git.pullRebase", |c, _| {
        c.git_pull(Some(PullMode::Rebase));
        CommandOutcome::Done
    }));
    out.push(write("git.pullMerge", |c, _| {
        c.git_pull(Some(PullMode::Merge));
        CommandOutcome::Done
    }));
    out.push(write("git.push", |c, _| {
        c.git_push();
        CommandOutcome::Done
    }));
    out.push(
        write("git.sync", |c, _| {
            c.git_sync(false);
            CommandOutcome::Done
        })
        .icon("refresh-cw"),
    );
    out.push(
        CommandSpec::new(
            "git.cancel",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.git_cancel();
                CommandOutcome::Done
            },
        )
        .when("gitNetwork")
        .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "git.syncNotes",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| {
                c.git_sync_notes();
                CommandOutcome::Done
            },
        )
        .when("gitRepo && notesSync && !gitBusy")
        .expect("valid"),
    );

    // Clone / init
    out.push(
        CommandSpec::new(
            "git.clone",
            Category::Git,
            |c: &mut AppCore, i: &CommandInvocation| {
                match arg(i, "url") {
                    Some(url) => c.git_clone(url.to_owned()),
                    None => c.effects.push(Effect::OpenPicker(PickerKind::GitCloneUrl)),
                }
                CommandOutcome::Done
            },
        )
        .when("!gitNetwork")
        .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "git.init",
            Category::Git,
            |c: &mut AppCore, _: &CommandInvocation| {
                if let Some(root) = c.workspace.as_ref().map(|w| w.root.clone()) {
                    c.files
                        .prompts
                        .push_back(crate::files::Prompt::Git(GitPrompt::Init(root)));
                }
                CommandOutcome::Done
            },
        )
        .when("hasWorkspace && !gitRepo")
        .expect("valid"),
    );
}

fn show(core: &mut AppCore, panel: PanelId) -> CommandOutcome {
    core.exit_maximize();
    core.panels.show(panel);
    CommandOutcome::Done
}

fn message(inv: &CommandInvocation) -> Option<String> {
    arg(inv, "message").map(str::to_owned)
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

/// Work-tree-relative paths: `{ "paths": [rel…] }`, else the target file.
fn target_rels(core: &AppCore, inv: &CommandInvocation) -> Vec<String> {
    if let Some(list) = inv.args.get("paths").and_then(|v| v.as_array()) {
        return list
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
    }
    target_path(core, inv)
        .and_then(|p| core.git_rel(&p))
        .into_iter()
        .collect()
}

fn paths_op(
    core: &mut AppCore,
    inv: &CommandInvocation,
    op: fn(Vec<String>) -> LocalOp,
) -> CommandOutcome {
    let rels = target_rels(core, inv);
    if rels.is_empty() {
        return bad_args(inv);
    }
    core.git_submit(op(rels));
    CommandOutcome::Done
}

fn discard(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let rels = target_rels(core, inv);
    if rels.is_empty() {
        return bad_args(inv);
    }
    core.git_discard(rels);
    CommandOutcome::Done
}

/// `{ "message"?, "amend"? }`; without a message, the Source Control draft.
fn commit(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let amend = inv
        .args
        .get("amend")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    core.git_commit(message(inv), amend, false, false);
    CommandOutcome::Done
}

/// `{ "name", "from"? }`; without a name, asks for one.
fn create_branch(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "name") {
        Some(name) => {
            let from = arg(inv, "from")
                .map(str::to_owned)
                .or_else(|| core.git.create_from.take());
            core.git_submit(LocalOp::CreateBranch {
                name: name.trim().to_owned(),
                from,
            });
        }
        None => {
            core.git.create_from = None;
            core.effects
                .push(Effect::OpenPicker(PickerKind::GitBranchName));
        }
    }
    CommandOutcome::Done
}

/// A row of the branch picker.
fn branch_pick(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(value) = arg(inv, "value").map(str::to_owned) else {
        return bad_args(inv);
    };
    match (value.as_str(), core.git.branch_pick) {
        (":create", _) => {
            core.git.create_from = None;
            core.effects
                .push(Effect::OpenPicker(PickerKind::GitBranchName));
        }
        (":createFrom", _) => core.git_pick_branch(BranchPick::CreateFrom),
        (_, BranchPick::CreateFrom) => {
            core.git.create_from = Some(value);
            core.effects
                .push(Effect::OpenPicker(PickerKind::GitBranchName));
        }
        (_, BranchPick::Switch) => {
            if !core
                .git
                .branches
                .iter()
                .any(|b| b.name == value && b.current)
            {
                core.git_checkout(value);
            }
        }
    }
    CommandOutcome::Done
}

/// `{ "line", "view"? }`: a gutter bar was clicked (toggles the peek).
fn peek_change(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(line) = inv.args.get("line").and_then(|v| v.as_u64()) else {
        return bad_args(inv);
    };
    let view = match inv.args.get("view").and_then(|v| v.as_u64()) {
        Some(id) => Some(ViewId(id)),
        None => core.workbench.active_editor(),
    };
    let Some(view) = view else {
        return bad_args(inv);
    };
    let line = line as usize;
    let same = core
        .git
        .peek
        .as_ref()
        .is_some_and(|p| p.view == view && (p.buf.contains(&line) || p.buf.start == line));
    if same {
        core.git.peek = None;
    } else {
        core.git_peek(view, line);
    }
    CommandOutcome::Done
}

/// `{ "path": work tree }`: Source Control switches to that repository.
fn select_repository(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "path") {
        Some(p) if core.git_select_repository(std::path::Path::new(p)) => CommandOutcome::Done,
        _ => bad_args(inv),
    }
}

/// `{ "branch" }`.
fn checkout(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "branch") {
        Some(b) if core.git.repo.is_some() => {
            core.git_checkout(b.to_owned());
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

/// `{ "path": rel, "staged": bool }`.
fn open_change(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(rel) = arg(inv, "path").map(str::to_owned) else {
        return bad_args(inv);
    };
    let staged = inv
        .args
        .get("staged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    core.git_open_change(&rel, staged);
    CommandOutcome::Done
}

/// View File Changes: the active file's changes against the index.
fn view_file_changes(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match target_path(core, inv).and_then(|p| core.git_rel(&p)) {
        Some(rel) => {
            core.git_open_change(&rel, false);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

/// `{ "path": absolute }`: shows the file in the Explorer.
fn reveal(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(path) = arg(inv, "path").map(PathBuf::from) else {
        return bad_args(inv);
    };
    core.exit_maximize();
    core.panels.show(PanelId::Explorer);
    core.reveal_in_explorer(&path, true);
    CommandOutcome::Done
}

fn compare_with_head(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(path) = target_path(core, inv) else {
        return bad_args(inv);
    };
    let right = core.git_current_source(&path);
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
            let right = core.git_current_source(&h.path);
            core.compare(CompareSource::GitRevision(h.path, rev), right, false);
        }
        HistoryMode::OpenAtRevision => {
            core.git_read_blob(BlobPurpose::Open { path: h.path, rev });
        }
    }
    CommandOutcome::Done
}
