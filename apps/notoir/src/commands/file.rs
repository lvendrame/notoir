//! File and tab commands (docs/specs/04, 08, 09). Commands that act on a tab
//! take an optional `{ "view": id }` (context menus pass the clicked tab) or
//! `{ "doc": id }`; without one they act on the active tab.

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::keymap::KeyBindingDef;
use notoir_editor::LineEnding;
use notoir_editor::encoding::TextEncoding;

use super::{Specs, arg, bad_args};
use crate::core::{AppCore, Effect};
use crate::files::Prompt;
use crate::shell::overlays::PickerKind;

pub(super) fn specs(out: &mut Specs) {
    let editor = "hasActiveEditor";
    let k = KeyBindingDef::new;
    out.push(
        CommandSpec::new("file.open", Category::File, open)
            .keys([k("Mod+O")])
            .icon("folder-open"),
    );
    out.push(CommandSpec::new(
        "file.newFromTemplate",
        Category::File,
        |c: &mut AppCore, inv: &CommandInvocation| {
            let dir = c
                .workspace
                .as_ref()
                .map(|ws| ws.target_dir(c.explorer_target(inv).as_deref()));
            c.templates_start(dir);
            CommandOutcome::Done
        },
    ));
    out.push(
        CommandSpec::new(
            "file.pickTemplate",
            Category::File,
            |c: &mut AppCore, inv: &CommandInvocation| match super::arg(inv, "path") {
                Some(p) => {
                    let p = p.to_owned();
                    c.templates_pick(&p);
                    CommandOutcome::Done
                }
                None => super::bad_args(inv),
            },
        )
        .hidden(),
    );
    out.push(
        CommandSpec::new("file.newScratchpad", Category::File, new_scratchpad)
            .keys([k("Mod+Alt+N")]),
    );
    out.push(
        CommandSpec::new("file.newWindow", Category::File, new_window).keys([k("Mod+Shift+N")]),
    );
    out.push(
        CommandSpec::new("file.save", Category::File, save)
            .keys([k("Mod+S")])
            .when(editor)
            .expect("valid")
            .icon("save"),
    );
    out.push(
        CommandSpec::new("file.saveAs", Category::File, save_as)
            .keys([k("Mod+Shift+S")])
            .when(editor)
            .expect("valid"),
    );
    out.push(CommandSpec::new("file.saveAsAdmin", Category::File, save_as_admin).hidden());
    out.push(CommandSpec::new("file.saveAll", Category::File, save_all).keys([k("Mod+Alt+S")]));
    out.push(
        CommandSpec::new("file.revert", Category::File, revert)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.close", Category::File, close)
            .keys([k("Mod+W")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.closeOthers", Category::File, close_others)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.closeToRight", Category::File, close_to_right)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.closeSaved", Category::File, close_saved)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.closeAll", Category::File, close_all)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.copyPath", Category::File, copy_path)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("file.reveal", Category::File, reveal)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("view.togglePin", Category::View, toggle_pin)
            .when(editor)
            .expect("valid")
            .icon("pin"),
    );
    out.push(
        CommandSpec::new("view.duplicate", Category::View, duplicate)
            .when(editor)
            .expect("valid"),
    );
    out.push(CommandSpec::new("view.activate", Category::View, activate).hidden());
    out.push(
        CommandSpec::new("view.reopenClosedTab", Category::View, reopen_closed)
            .keys([k("Mod+Shift+T")]),
    );
    out.push(
        CommandSpec::new("navigate.nextTab", Category::Navigate, next_tab)
            .keys([k("Ctrl+PageDown").mac("Cmd+Alt+Right")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("navigate.previousTab", Category::Navigate, previous_tab)
            .keys([k("Ctrl+PageUp").mac("Cmd+Alt+Left")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("navigate.switchTab", Category::Navigate, switch_tab)
            .keys([k("Ctrl+Tab")])
            .when(editor)
            .expect("valid"),
    );
    out.push(CommandSpec::new("file.openRecent", Category::File, open_recent).hidden());
    out.push(CommandSpec::new("file.openScratchpad", Category::File, open_scratchpad).hidden());
    out.push(CommandSpec::new("file.removeRecent", Category::File, remove_recent).hidden());
    out.push(CommandSpec::new("file.pinRecent", Category::File, pin_recent).hidden());
    out.push(CommandSpec::new(
        "file.clearRecent",
        Category::File,
        clear_recent,
    ));
    out.push(
        CommandSpec::new(
            "editor.changeLineEnding",
            Category::Edit,
            change_line_ending,
        )
        .when(editor)
        .expect("valid"),
    );
    out.push(
        CommandSpec::new("editor.changeEncoding", Category::Edit, change_encoding)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("editor.changeLanguageMode", Category::Edit, change_language)
            .when(editor)
            .expect("valid"),
    );
}

fn view_of(core: &AppCore, inv: &CommandInvocation) -> Option<ViewId> {
    match inv.args.get("view").and_then(|v| v.as_u64()) {
        Some(id) => core.workbench.view(ViewId(id)).map(|v| v.id),
        None => core.workbench.active_view(),
    }
}

fn doc_of(core: &AppCore, inv: &CommandInvocation) -> Option<DocumentId> {
    if let Some(id) = inv.args.get("doc").and_then(|v| v.as_u64()) {
        return core.workbench.document(DocumentId(id)).map(|d| d.id);
    }
    view_of(core, inv)
        .and_then(|v| core.workbench.view(v))
        .map(|v| v.document_id)
}

fn open(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.open_dialog();
    CommandOutcome::Done
}

fn new_scratchpad(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.new_scratchpad();
    CommandOutcome::Done
}

fn new_window(_: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).arg("--new-window").spawn() {
                tracing::error!(error = %e, "could not start a new window");
            }
        }
        Err(e) => tracing::error!(error = %e, "no executable path"),
    }
    CommandOutcome::Done
}

fn save(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(doc) = doc_of(core, inv) {
        core.save(doc);
    }
    CommandOutcome::Done
}

fn save_as(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(doc) = doc_of(core, inv) {
        core.save_as(doc);
    }
    CommandOutcome::Done
}

fn save_as_admin(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(doc) = doc_of(core, inv) {
        core.save_as_admin(doc);
    }
    CommandOutcome::Done
}

fn save_all(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.save_all();
    CommandOutcome::Done
}

fn revert(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(doc) = doc_of(core, inv).filter(|d| {
        core.workbench
            .document(*d)
            .is_some_and(|d| d.path().is_some())
    }) {
        core.files.prompts.push_back(Prompt::Revert { doc });
    }
    CommandOutcome::Done
}

fn close(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = view_of(core, inv) {
        core.request_close(v);
    }
    CommandOutcome::Done
}

/// Tabs of the clicked tab's group, in strip order.
fn group_tabs(core: &AppCore, view: ViewId) -> Vec<ViewId> {
    core.workbench
        .layout
        .group_of(view)
        .and_then(|g| core.workbench.layout.group(g))
        .map(|g| g.tabs.clone())
        .unwrap_or_default()
}

fn pinned(core: &AppCore, view: ViewId) -> bool {
    core.workbench.view(view).is_some_and(|v| v.pinned)
}

fn close_others(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(keep) = view_of(core, inv) else {
        return CommandOutcome::Done;
    };
    let targets: Vec<ViewId> = group_tabs(core, keep)
        .into_iter()
        .filter(|v| *v != keep && !pinned(core, *v))
        .collect();
    core.request_close_many(targets);
    CommandOutcome::Done
}

fn close_to_right(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(from) = view_of(core, inv) else {
        return CommandOutcome::Done;
    };
    let tabs = group_tabs(core, from);
    let idx = tabs.iter().position(|v| *v == from).unwrap_or(0);
    let targets: Vec<ViewId> = tabs
        .into_iter()
        .skip(idx + 1)
        .filter(|v| !pinned(core, *v))
        .collect();
    core.request_close_many(targets);
    CommandOutcome::Done
}

fn close_saved(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(any) = view_of(core, inv) else {
        return CommandOutcome::Done;
    };
    let targets: Vec<ViewId> = group_tabs(core, any)
        .into_iter()
        .filter(|v| !pinned(core, *v))
        .filter(|v| {
            core.workbench
                .view(*v)
                .and_then(|v| core.workbench.document(v.document_id))
                .is_some_and(|d| !d.is_dirty())
        })
        .collect();
    core.request_close_many(targets);
    CommandOutcome::Done
}

fn close_all(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(any) = view_of(core, inv) else {
        return CommandOutcome::Done;
    };
    let keep_pinned = core
        .settings
        .bool("workbench.tabs.closeAllKeepsPinned", None);
    let targets: Vec<ViewId> = group_tabs(core, any)
        .into_iter()
        .filter(|v| !(keep_pinned && pinned(core, *v)))
        .collect();
    core.request_close_many(targets);
    CommandOutcome::Done
}

fn copy_path(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = doc_of(core, inv)
        .and_then(|d| core.workbench.document(d))
        .and_then(|d| d.path())
    {
        core.effects
            .push(Effect::CopyText(path.display().to_string()));
    }
    CommandOutcome::Done
}

fn reveal(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(path) = doc_of(core, inv)
        .and_then(|d| core.workbench.document(d))
        .and_then(|d| d.path())
        && let Err(e) = notoir_platform::reveal::reveal(path)
    {
        tracing::warn!(error = %e, "reveal failed");
    }
    CommandOutcome::Done
}

fn toggle_pin(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = view_of(core, inv) {
        let next = !pinned(core, v);
        let _ = core.workbench.set_pinned(v, next);
    }
    CommandOutcome::Done
}

fn duplicate(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = view_of(core, inv)
        && let Ok(new) = core.workbench.duplicate_view(v)
    {
        core.effects.push(Effect::FocusEditor(new));
    }
    CommandOutcome::Done
}

fn activate(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = view_of(core, inv) {
        let _ = core.workbench.activate_view(v);
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}

fn reopen_closed(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.reopen_closed();
    CommandOutcome::Done
}

fn cycle(core: &mut AppCore, delta: isize) {
    let Some(active) = core.workbench.active_view() else {
        return;
    };
    let tabs = group_tabs(core, active);
    let n = tabs.len() as isize;
    if n < 2 {
        return;
    }
    let i = tabs.iter().position(|v| *v == active).unwrap_or(0) as isize;
    let next = tabs[((i + delta).rem_euclid(n)) as usize];
    let _ = core.workbench.activate_view(next);
    core.effects.push(Effect::FocusEditor(next));
}

fn next_tab(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    cycle(core, 1);
    CommandOutcome::Done
}

fn previous_tab(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    cycle(core, -1);
    CommandOutcome::Done
}

/// Ctrl+Tab: the previously active tab (MRU).
fn switch_tab(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let previous = core.workbench.layout.active().mru.get(1).copied();
    if let Some(v) = previous {
        let _ = core.workbench.activate_view(v);
        core.effects.push(Effect::FocusEditor(v));
    }
    CommandOutcome::Done
}

fn open_recent(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "path") {
        Some(p) => {
            core.open_path(std::path::Path::new(p), crate::files::OpenAt::default());
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn open_scratchpad(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "id") {
        Some(id) => {
            let id = id.to_owned();
            core.open_scratchpad(&id);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn remove_recent(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match arg(inv, "path") {
        Some(p) => {
            let p = p.to_owned();
            core.remove_recent_file(&p);
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
            let p = p.to_owned();
            core.set_recent_pinned(&p, pinned);
            CommandOutcome::Done
        }
        None => bad_args(inv),
    }
}

fn clear_recent(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.clear_recent();
    CommandOutcome::Done
}

/// `{ "eol": "lf" | "crlf" | "cr" }`, or a picker without args.
fn change_line_ending(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(doc) = doc_of(core, inv) else {
        return CommandOutcome::Done;
    };
    let eol = match arg(inv, "eol") {
        None => {
            core.effects
                .push(Effect::OpenPicker(PickerKind::LineEnding));
            return CommandOutcome::Done;
        }
        Some("lf") => LineEnding::Lf,
        Some("crlf") => LineEnding::CrLf,
        Some("cr") => LineEnding::Cr,
        Some(_) => return bad_args(inv),
    };
    core.set_line_ending(doc, eol);
    CommandOutcome::Done
}

/// Parses an encoding picker value (`utf-8-bom` or a WHATWG label).
pub fn parse_encoding(value: &str) -> Option<TextEncoding> {
    if value == "utf-8-bom" {
        return Some(TextEncoding {
            encoding: encoding_rs::UTF_8,
            bom: true,
        });
    }
    let encoding = encoding_rs::Encoding::for_label(value.as_bytes())?;
    let bom = encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE;
    Some(TextEncoding { encoding, bom })
}

/// `{ "action": "reopen" | "save", "encoding": label }`; without arguments a
/// two-step picker.
fn change_encoding(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(doc) = doc_of(core, inv) else {
        return CommandOutcome::Done;
    };
    let reopen = match arg(inv, "action") {
        None => {
            core.effects
                .push(Effect::OpenPicker(PickerKind::EncodingAction));
            return CommandOutcome::Done;
        }
        Some("reopen") => true,
        Some("save") => false,
        Some(_) => return bad_args(inv),
    };
    let Some(value) = arg(inv, "encoding") else {
        core.effects
            .push(Effect::OpenPicker(PickerKind::Encoding { reopen }));
        return CommandOutcome::Done;
    };
    let Some(encoding) = parse_encoding(value) else {
        return bad_args(inv);
    };
    if reopen {
        core.reopen_with_encoding(doc, encoding);
    } else {
        core.set_save_encoding(doc, encoding);
    }
    CommandOutcome::Done
}

/// `{ "language": id }`, or a picker without args.
fn change_language(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let Some(doc) = doc_of(core, inv) else {
        return CommandOutcome::Done;
    };
    match arg(inv, "language") {
        None => {
            core.effects
                .push(Effect::OpenPicker(PickerKind::LanguageMode));
            CommandOutcome::Done
        }
        Some(id) => match notoir_editor::LanguageId::from_id(id) {
            Some(lang) => {
                core.set_language(doc, lang);
                CommandOutcome::Done
            }
            None => bad_args(inv),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_labels() {
        assert_eq!(parse_encoding("utf-8-bom").map(|e| e.bom), Some(true));
        assert_eq!(
            parse_encoding("utf-16le").map(|e| (e.encoding.name(), e.bom)),
            Some(("UTF-16LE", true))
        );
        assert_eq!(
            parse_encoding("shift_jis").map(|e| e.encoding.name()),
            Some("Shift_JIS")
        );
        assert!(parse_encoding("klingon").is_none());
    }
}
