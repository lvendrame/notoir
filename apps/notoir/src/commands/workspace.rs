//! Workspace and Explorer commands (docs/specs/07, 08). Explorer commands take
//! an optional `{ "path": "…" }` (context menus pass the clicked entry); without
//! one they act on the selected entry.

use std::path::{Path, PathBuf};

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::DocumentId;
use notoir_core::keymap::KeyBindingDef;
use notoir_core::settings::SettingValue;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect};
use crate::workspace::EditKind;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    let ws = "hasWorkspace";
    let tree = "explorerFocus";
    out.push(
        CommandSpec::new("workspace.open", Category::File, open)
            .keys([k("Mod+K Mod+O")])
            .icon("folder-open"),
    );
    out.push(
        CommandSpec::new("workspace.close", Category::File, close)
            .when(ws)
            .expect("valid"),
    );
    out.push(CommandSpec::new("workspace.openRecent", Category::File, open_recent).hidden());
    out.push(CommandSpec::new("workspace.removeRecent", Category::File, remove_recent).hidden());
    out.push(CommandSpec::new("workspace.pinRecent", Category::File, pin_recent).hidden());
    out.push(
        CommandSpec::new("workspace.trust", Category::File, trust)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("workspace.trustLater", Category::File, |_, _| {
            CommandOutcome::Done
        })
        .hidden(),
    );
    out.push(
        CommandSpec::new("explorer.newFile", Category::File, new_file)
            .when(ws)
            .expect("valid")
            .icon("file-plus"),
    );
    out.push(
        CommandSpec::new("explorer.newFolder", Category::File, new_folder)
            .when(ws)
            .expect("valid")
            .icon("folder-plus"),
    );
    out.push(
        CommandSpec::new("explorer.rename", Category::File, rename)
            .keys([k("F2").when(tree)])
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.duplicate", Category::File, duplicate)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.delete", Category::File, delete)
            .keys([k("Delete").when(tree), k("Mod+Backspace").when(tree)])
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.copy", Category::File, copy)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.paste", Category::File, paste)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.copyPath", Category::File, copy_path)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "explorer.copyRelativePath",
            Category::File,
            copy_relative_path,
        )
        .when(ws)
        .expect("valid"),
    );
    out.push(
        CommandSpec::new(
            "file.copyRelativePath",
            Category::File,
            copy_relative_path_of_doc,
        )
        .when("hasActiveEditor && hasWorkspace")
        .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.reveal", Category::File, reveal_in_os)
            .when(ws)
            .expect("valid"),
    );
    out.push(CommandSpec::new("explorer.open", Category::File, open_entry).hidden());
    out.push(
        CommandSpec::new("explorer.openToSide", Category::File, open_to_side)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.refresh", Category::File, refresh)
            .when(ws)
            .expect("valid")
            .icon("refresh-cw"),
    );
    out.push(
        CommandSpec::new("explorer.collapseAll", Category::File, collapse_all)
            .when(ws)
            .expect("valid")
            .icon("chevrons-down-up"),
    );
    out.push(CommandSpec::new(
        "explorer.toggleHidden",
        Category::View,
        toggle_hidden,
    ));
    out.push(CommandSpec::new(
        "explorer.toggleGitIgnored",
        Category::View,
        toggle_ignored,
    ));
    out.push(CommandSpec::new("explorer.sortOrder", Category::View, sort_order).hidden());
    out.push(
        CommandSpec::new("explorer.focusFilter", Category::View, focus_filter)
            .keys([k("Mod+Alt+F")])
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.undo", Category::Edit, undo)
            .when(ws)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("explorer.revealActive", Category::View, reveal_active)
            .when("hasActiveEditor && hasWorkspace")
            .expect("valid"),
    );
    out.push(CommandSpec::new("explorer.drop", Category::File, drop).hidden());
}

fn open(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "path") {
        Some(p) => core.open_workspace(Path::new(p)),
        None => core.open_workspace_dialog(),
    }
    CommandOutcome::Done
}

fn close(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.close_workspace(None);
    CommandOutcome::Done
}

fn open_recent(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "path") {
        Some(p) => {
            core.open_workspace(Path::new(p));
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn remove_recent(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "path") {
        Some(p) => {
            core.remove_recent_workspace(p);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn pin_recent(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let pinned = inv
        .args
        .get("pinned")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    match arg(inv, "path") {
        Some(p) => {
            core.set_recent_workspace_pinned(p, pinned);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn trust(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.trust_workspace();
    CommandOutcome::Done
}

fn target_dir(core: &AppCore, inv: &CommandInvocation) -> Option<PathBuf> {
    let ws = core.workspace.as_ref()?;
    Some(ws.target_dir(core.explorer_target(inv).as_deref()))
}

fn new_file(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(dir) = target_dir(core, inv) {
        core.begin_edit(EditKind::NewFile {
            dir,
            template: None,
        });
    }
    CommandOutcome::Done
}

fn new_folder(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(dir) = target_dir(core, inv) {
        core.begin_edit(EditKind::NewFolder { dir });
    }
    CommandOutcome::Done
}

/// The target entry, never the workspace root itself.
fn entry(core: &AppCore, inv: &CommandInvocation) -> Option<PathBuf> {
    let p = core.explorer_target(inv)?;
    let ws = core.workspace.as_ref()?;
    (p != ws.root && ws.contains(&p)).then_some(p)
}

fn rename(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = entry(core, inv) {
        core.begin_edit(EditKind::Rename { path });
    }
    CommandOutcome::Done
}

fn duplicate(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = entry(core, inv) {
        core.duplicate_path(path);
    }
    CommandOutcome::Done
}

fn delete(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = entry(core, inv) {
        core.request_delete(vec![path]);
    }
    CommandOutcome::Done
}

fn copy(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = entry(core, inv) {
        core.effects
            .push(Effect::CopyText(path.display().to_string()));
        if let Some(ws) = core.workspace.as_mut() {
            ws.clipboard = Some(path);
        }
    }
    CommandOutcome::Done
}

fn paste(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let src = core.workspace.as_ref().and_then(|w| w.clipboard.clone());
    if let (Some(src), Some(dir)) = (src, target_dir(core, inv)) {
        core.transfer_into(vec![src], dir, true);
    }
    CommandOutcome::Done
}

fn copy_path(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = core.explorer_target(inv) {
        core.effects
            .push(Effect::CopyText(path.display().to_string()));
    }
    CommandOutcome::Done
}

fn copy_relative_path(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(rel) = core
        .explorer_target(inv)
        .and_then(|p| core.workspace.as_ref()?.relative(&p))
    {
        core.effects.push(Effect::CopyText(rel));
    }
    CommandOutcome::Done
}

fn copy_relative_path_of_doc(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let doc = match (
        inv.args.get("doc").and_then(|v| v.as_u64()),
        inv.args.get("view").and_then(|v| v.as_u64()),
    ) {
        (Some(id), _) => Some(DocumentId(id)),
        (None, Some(v)) => core
            .workbench
            .view(notoir_core::ids::ViewId(v))
            .map(|v| v.document_id),
        _ => core.workbench.active_document(),
    };
    if let Some(rel) = doc
        .and_then(|d| core.workbench.document(d))
        .and_then(|d| d.path())
        .and_then(|p| core.workspace.as_ref()?.relative(p))
    {
        core.effects.push(Effect::CopyText(rel));
    }
    CommandOutcome::Done
}

fn reveal_in_os(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = core.explorer_target(inv)
        && let Err(e) = notoir_platform::reveal::reveal(&path)
    {
        tracing::warn!(error = %e, "reveal failed");
    }
    CommandOutcome::Done
}

fn open_entry(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(path) = core.explorer_target(inv) else {
        return bad_args(inv);
    };
    if let Some(ws) = core.workspace.as_mut() {
        ws.tree.selected = Some(path.clone());
    }
    let keep_focus = inv
        .args
        .get("preserveFocus")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    core.open_path(
        &path,
        crate::files::OpenAt {
            keep_focus,
            ..Default::default()
        },
    );
    CommandOutcome::Done
}

fn open_to_side(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(path) = entry(core, inv) else {
        return CommandOutcome::Done;
    };
    let side = core.workbench.side_group();
    core.workbench.layout.active_group = side;
    core.open_path(&path, crate::files::OpenAt::default());
    CommandOutcome::Done
}

fn refresh(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.refresh_explorer();
    CommandOutcome::Done
}

fn collapse_all(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(ws) = core.workspace.as_mut() {
        ws.tree.collapse_all();
    }
    CommandOutcome::Done
}

fn toggle_bool(core: &mut AppCore, key: &str) {
    let next = !core.settings.bool(key, None);
    core.set_user_setting(key, Some(SettingValue::Bool(next)));
}

fn toggle_hidden(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_bool(core, "explorer.showHidden");
    CommandOutcome::Done
}

fn toggle_ignored(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_bool(core, "explorer.showGitIgnored");
    CommandOutcome::Done
}

fn sort_order(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "value") {
        Some(v @ ("default" | "mixed" | "modified")) => {
            core.set_user_setting("explorer.sortOrder", Some(SettingValue::Str(v.into())));
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

fn focus_filter(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.panels.show(notoir_core::panels::PanelId::Explorer);
    if let Some(ws) = core.workspace.as_mut() {
        ws.filter_open = true;
        ws.filter_focus = true;
    }
    CommandOutcome::Done
}

fn undo(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.undo_explorer_op();
    CommandOutcome::Done
}

fn reveal_active(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.panels.show(notoir_core::panels::PanelId::Explorer);
    if let Some(path) = core
        .workbench
        .active_document()
        .and_then(|d| core.workbench.document(d))
        .and_then(|d| d.path())
        .map(Path::to_path_buf)
    {
        core.reveal_in_explorer(&path, true);
    }
    CommandOutcome::Done
}

/// `{ srcs: [paths], dir, copy }`: tree drag and drop and OS drops.
fn drop(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let srcs: Vec<PathBuf> = inv
        .args
        .get("srcs")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    let Some(dir) = arg(inv, "dir").map(PathBuf::from) else {
        return bad_args(inv);
    };
    let copy = inv
        .args
        .get("copy")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let inside = core
        .workspace
        .as_ref()
        .is_some_and(|w| srcs.iter().all(|s| w.contains(s)));
    if inside {
        core.request_transfer(srcs, dir, copy);
    } else {
        // Files from the OS are copied in.
        core.transfer_into(srcs, dir, true);
    }
    CommandOutcome::Done
}
