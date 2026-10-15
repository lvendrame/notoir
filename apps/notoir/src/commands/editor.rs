//! Editor commands: new file, undo/redo, clipboard, select all, word wrap,
//! zoom and rendering toggles (docs/specs/11, 14).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::document::{DocumentOrigin, LoadedText};
use notoir_core::keymap::KeyBindingDef;
use notoir_core::settings::SettingValue;
use notoir_core::workbench::TabPosition;
use notoir_editor::edit_ops;
use notoir_editor::{EditKind, LanguageId, Range, Selection};

use super::Specs;
use crate::core::{AppCore, Effect};
use crate::editor_widget::zoom_step;

pub(super) fn specs(out: &mut Specs) {
    let editor = "hasActiveEditor";
    out.push(
        CommandSpec::new("file.newUntitled", Category::File, new_untitled)
            .keys([KeyBindingDef::new("Mod+N")])
            .icon("file-plus"),
    );
    out.push(
        CommandSpec::new("edit.undo", Category::Edit, undo)
            .keys([KeyBindingDef::new("Mod+Z")])
            .when(editor)
            .expect("valid")
            .icon("undo-2"),
    );
    out.push(
        CommandSpec::new("edit.redo", Category::Edit, redo)
            .keys([
                KeyBindingDef::new("Mod+Shift+Z"),
                KeyBindingDef::new("Ctrl+Y").mac("Cmd+Shift+Z"),
            ])
            .when(editor)
            .expect("valid")
            .icon("redo-2"),
    );
    out.push(
        CommandSpec::new("edit.cut", Category::Edit, cut)
            .when(editor)
            .expect("valid")
            .icon("scissors"),
    );
    out.push(
        CommandSpec::new("edit.copy", Category::Edit, copy)
            .when(editor)
            .expect("valid")
            .icon("copy"),
    );
    out.push(
        CommandSpec::new("edit.paste", Category::Edit, paste)
            .when("hasActiveEditor && !readOnly")
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("edit.selectAll", Category::Edit, select_all)
            .keys([KeyBindingDef::new("Mod+A").when("editorFocus")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("view.toggleWordWrap", Category::View, toggle_wrap)
            .keys([KeyBindingDef::new("Alt+Z")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("view.zoomIn", Category::View, zoom_in)
            .keys([KeyBindingDef::new("Mod++")])
            .icon("zoom-in"),
    );
    out.push(
        CommandSpec::new("view.zoomOut", Category::View, zoom_out)
            .keys([KeyBindingDef::new("Mod+-")])
            .icon("zoom-out"),
    );
    out.push(
        CommandSpec::new("view.zoomReset", Category::View, zoom_reset)
            .keys([KeyBindingDef::new("Mod+0")]),
    );
    out.push(CommandSpec::new(
        "view.toggleLineNumbers",
        Category::View,
        toggle_line_numbers,
    ));
    out.push(CommandSpec::new(
        "view.toggleRelativeLineNumbers",
        Category::View,
        toggle_relative_line_numbers,
    ));
    out.push(CommandSpec::new("editor.copyLineNumber", Category::Edit, copy_line_number).hidden());
    out.push(CommandSpec::new(
        "view.toggleWhitespace",
        Category::View,
        toggle_whitespace,
    ));
    out.push(CommandSpec::new(
        "view.toggleCurrentLineHighlight",
        Category::View,
        toggle_line_highlight,
    ));
}

fn new_untitled(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let n = core.workbench.next_untitled_number();
    let doc = core.workbench.open_document(
        DocumentOrigin::Untitled { n },
        LoadedText::empty(LanguageId::PLAINTEXT),
    );
    let group = core.workbench.layout.active_group;
    match core
        .workbench
        .open_view(doc, group, TabPosition::AfterActive)
    {
        Ok(view) => {
            core.effects.push(Effect::FocusEditor(view));
            CommandOutcome::Done
        }
        Err(e) => {
            tracing::error!(error = %e, "could not open view");
            CommandOutcome::Done
        }
    }
}

fn active(core: &AppCore) -> Option<notoir_core::ids::ViewId> {
    core.workbench.active_view()
}

fn undo(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        let _ = core.workbench.undo(v);
    }
    CommandOutcome::Done
}

fn redo(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        let _ = core.workbench.redo(v);
    }
    CommandOutcome::Done
}

/// Text the clipboard commands act on, and the selection to delete for cut.
fn clipboard_source(core: &AppCore) -> Option<(notoir_core::ids::ViewId, String, Selection)> {
    let v = core.workbench.view(active(core)?)?;
    let rope = core.workbench.document(v.document_id)?.buffer.rope();
    let source =
        if v.selection.is_empty() && core.settings.bool("editor.emptySelectionClipboard", None) {
            edit_ops::line_ranges(rope, &v.selection)
        } else {
            v.selection.clone()
        };
    Some((v.id, edit_ops::selected_text(rope, &source), source))
}

fn copy(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some((_, text, _)) = clipboard_source(core)
        && !text.is_empty()
    {
        core.effects.push(Effect::CopyText(text));
    }
    CommandOutcome::Done
}

fn cut(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some((view, text, source)) = clipboard_source(core)
        && !text.is_empty()
    {
        core.effects.push(Effect::CopyText(text));
        if let Some(res) = edit_ops::insert(&source, "", EditKind::Other) {
            let _ = core
                .workbench
                .edit(view, res.changes, res.selection, res.kind, core.now_ms);
        }
    }
    CommandOutcome::Done
}

fn paste(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let Some(view) = active(core) else {
        return CommandOutcome::Done;
    };
    let text = match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(t) => notoir_editor::line_ending::normalize(&t),
        Err(e) => {
            tracing::warn!(error = %e, "clipboard unavailable");
            return CommandOutcome::Done;
        }
    };
    let sel = core
        .workbench
        .view(view)
        .map(|v| v.selection.clone())
        .unwrap_or_default();
    if let Some(res) = edit_ops::insert(&sel, &text, EditKind::Other) {
        let _ = core
            .workbench
            .edit(view, res.changes, res.selection, res.kind, core.now_ms);
    }
    CommandOutcome::Done
}

fn select_all(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        let len = core
            .workbench
            .view(v)
            .and_then(|v| core.workbench.document(v.document_id))
            .map_or(0, |d| d.buffer.len_chars());
        let _ = core
            .workbench
            .set_selection(v, Selection::single(Range::new(0, len)));
    }
    CommandOutcome::Done
}

fn toggle_wrap(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        let on = core.wrap_enabled(v);
        if let Some(view) = core.workbench.views.get_mut(v) {
            view.wrap_override = Some(!on);
        }
    }
    CommandOutcome::Done
}

fn set_zoom(core: &mut AppCore, f: impl Fn(u32) -> u32) -> CommandOutcome {
    let per_view = core.settings.str("editor.zoomScope", None) == "view";
    match active(core).filter(|_| per_view) {
        Some(v) => {
            let current = (core.zoom_for(v) * 100.0).round() as u32;
            if let Some(view) = core.workbench.views.get_mut(v) {
                view.zoom = Some(f(current) as f32 / 100.0);
            }
        }
        None => core.zoom = f(core.zoom),
    }
    CommandOutcome::Done
}

fn zoom_in(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    set_zoom(core, |z| zoom_step(z, true))
}

fn zoom_out(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    set_zoom(core, |z| zoom_step(z, false))
}

fn zoom_reset(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    set_zoom(core, |_| 100)
}

fn toggle_setting(core: &mut AppCore, key: &str, on: &str, off: &str) -> CommandOutcome {
    let next = if core.settings.str(key, None) == off {
        on
    } else {
        off
    };
    core.set_user_setting(key, Some(SettingValue::Str(next.to_owned())));
    CommandOutcome::Done
}

fn toggle_line_numbers(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_setting(core, "editor.lineNumbers", "on", "off")
}

fn toggle_relative_line_numbers(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_setting(core, "editor.lineNumbers", "relative", "on")
}

/// `{ line }` (0-based) → its number on the clipboard.
fn copy_line_number(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match inv.args.get("line").and_then(|v| v.as_u64()) {
        Some(l) => {
            core.effects
                .push(crate::core::Effect::CopyText((l + 1).to_string()));
            CommandOutcome::Done
        }
        None => super::bad_args(inv),
    }
}

fn toggle_whitespace(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let next = if core.settings.str("editor.renderWhitespace", None) == "all" {
        "selection"
    } else {
        "all"
    };
    core.set_user_setting(
        "editor.renderWhitespace",
        Some(SettingValue::Str(next.to_owned())),
    );
    CommandOutcome::Done
}

fn toggle_line_highlight(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    toggle_setting(core, "editor.highlightCurrentLine", "all", "none")
}
