//! Markdown commands (docs/specs/24): display modes, editing aids, preview
//! actions.

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::DocumentId;
use notoir_core::keymap::KeyBindingDef;
use notoir_core::view::ViewKind;

use super::{Specs, arg, bad_args};
use crate::core::AppCore;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    let md = "hasActiveEditor && languageId == 'markdown'";
    let md_edit = "editorFocus && languageId == 'markdown'";
    let spec = |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, Category::Markdown, f)
            .when(md)
            .expect("valid")
    };
    out.push(spec("markdown.togglePreview", toggle_preview).keys([k("Mod+Shift+V")]));
    out.push(spec("markdown.openPreviewToSide", open_to_side).keys([k("Mod+K V")]));
    out.push(spec("markdown.showEditor", show_editor));
    out.push(spec("markdown.showSplit", open_to_side));
    out.push(spec("markdown.showPreview", show_preview));
    out.push(spec("markdown.exportHtml", |c, _| {
        if let Some(doc) = c.workbench.active_document() {
            c.markdown_export(doc);
        }
        CommandOutcome::Done
    }));
    let edit_spec =
        |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
            CommandSpec::new(id, Category::Markdown, f)
                .when("hasActiveEditor && languageId == 'markdown' && !readOnly")
                .expect("valid")
        };
    out.push(edit_spec("markdown.toggleStrikethrough", |c, i| {
        wrap(c, i, "~~")
    }));
    out.push(edit_spec("markdown.insertBlock", insert_block));
    out.push(edit_spec("markdown.insertImage", |c, i| {
        if let Some(v) = target_view(c, i) {
            c.open_md_insert_dialog(v, crate::markdown_insert::InsertDialogKind::Image);
        }
        CommandOutcome::Done
    }));
    out.push(edit_spec("markdown.insertCodePicker", |c, i| {
        if let Some(v) = target_view(c, i) {
            c.open_md_insert_dialog(v, crate::markdown_insert::InsertDialogKind::Code);
        }
        CommandOutcome::Done
    }));
    // Confirmation checks the captured target rather than the active editor.
    out.push(
        CommandSpec::new(
            "markdown.confirmInsert",
            Category::Markdown,
            |c: &mut AppCore, i| {
                c.confirm_md_insert(arg(i, "language"));
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
    out.push(edit_spec("markdown.toggleBold", toggle_bold).keys([k("Mod+B").when(md_edit)]));
    out.push(edit_spec("markdown.toggleItalic", toggle_italic).keys([k("Mod+I").when(md_edit)]));
    out.push(edit_spec("markdown.toggleCode", toggle_code));
    out.push(edit_spec("markdown.insertLink", insert_link));
    out.push(CommandSpec::new("markdown.toggleTask", Category::Markdown, toggle_task).hidden());
    out.push(CommandSpec::new("markdown.openLink", Category::Markdown, open_link).hidden());
    out.push(CommandSpec::new("markdown.revealSource", Category::Markdown, reveal_source).hidden());
}

fn view(core: &AppCore) -> Option<notoir_core::ids::ViewId> {
    core.workbench.active_view()
}

fn mode(core: &mut AppCore, m: &str) -> CommandOutcome {
    if let Some(v) = view(core) {
        core.set_markdown_mode(v, m);
    }
    CommandOutcome::Done
}

fn toggle_preview(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let current = view(core)
        .and_then(|v| core.workbench.view(v))
        .map(|v| v.kind);
    let next = if current == Some(ViewKind::MarkdownPreview) {
        "edit"
    } else {
        "preview"
    };
    mode(core, next)
}

fn open_to_side(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    mode(core, "split")
}

fn show_editor(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    mode(core, "edit")
}

fn show_preview(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    mode(core, "preview")
}

fn wrap(core: &mut AppCore, inv: &CommandInvocation, marker: &str) -> CommandOutcome {
    if let Some(v) = target_view(core, inv) {
        core.toggle_md_wrap(v, marker);
    }
    CommandOutcome::Done
}

fn toggle_bold(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    wrap(core, inv, "**")
}

fn toggle_italic(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    wrap(core, inv, "*")
}

fn toggle_code(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    wrap(core, inv, "`")
}

fn insert_link(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = target_view(core, inv) {
        core.insert_md_link(v);
    }
    CommandOutcome::Done
}

fn doc_arg(core: &AppCore, inv: &CommandInvocation) -> Option<DocumentId> {
    let id = inv.args.get("doc").and_then(|v| v.as_u64())?;
    core.workbench.document(DocumentId(id)).map(|d| d.id)
}

/// `{ doc, start, end }`: the task marker's char range.
fn toggle_task(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let start = inv.args.get("start").and_then(|v| v.as_u64());
    let end = inv.args.get("end").and_then(|v| v.as_u64());
    match (doc_arg(core, inv), start, end) {
        (Some(doc), Some(a), Some(b)) => {
            core.toggle_md_task(doc, (a as usize, b as usize));
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

/// `{ doc, url }`.
fn open_link(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match (doc_arg(core, inv), arg(inv, "url")) {
        (Some(doc), Some(url)) => {
            let url = url.to_owned();
            core.follow_md_link(doc, &url);
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

/// `{ doc, pos }`: double-click in the preview puts the editor caret there.
fn reveal_source(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let pos = inv.args.get("pos").and_then(|v| v.as_u64());
    let (Some(doc), Some(pos)) = (doc_arg(core, inv), pos) else {
        return bad_args(inv);
    };
    let Some(view) = core.workbench.views.of_document(doc).next().map(|v| v.id) else {
        return CommandOutcome::Done;
    };
    let pos = (pos as usize).min(
        core.workbench
            .document(doc)
            .map_or(0, |d| d.buffer.len_chars()),
    );
    let _ = core
        .workbench
        .set_selection(view, notoir_editor::Selection::caret(pos));
    if core
        .workbench
        .view(view)
        .is_some_and(|v| v.kind == ViewKind::MarkdownPreview)
    {
        core.set_markdown_mode(view, "split");
    }
    core.find.center.push(view);
    core.effects.push(crate::core::Effect::FocusEditor(view));
    CommandOutcome::Done
}

fn target_view(core: &AppCore, inv: &CommandInvocation) -> Option<notoir_core::ids::ViewId> {
    match inv.args.get("view") {
        Some(id) => id
            .as_u64()
            .map(notoir_core::ids::ViewId)
            .filter(|v| core.workbench.view(*v).is_some()),
        None => view(core),
    }
}

fn insert_block(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    use notoir_editor::markdown_edit::Insert;
    let Some(view) = target_view(core, inv) else {
        return bad_args(inv);
    };
    let kind = match arg(inv, "kind").unwrap_or("quote") {
        "heading" => match inv.args.get("level").and_then(|v| v.as_u64()) {
            Some(level @ 1..=6) => Insert::Heading(level as u8),
            _ => return bad_args(inv),
        },
        "quote" => Insert::Quote,
        "bullet" => Insert::Bullet,
        "numbered" => Insert::Numbered,
        "task" => Insert::Task,
        "table" => {
            core.open_md_insert_dialog(view, crate::markdown_insert::InsertDialogKind::Table);
            return CommandOutcome::Done;
        }
        "rule" => Insert::Rule,
        "code" => {
            let language = arg(inv, "language").unwrap_or("");
            if !language.is_empty() && notoir_editor::LanguageId::from_id(language).is_none() {
                return bad_args(inv);
            }
            Insert::Code(if language == "shellscript" {
                "sh"
            } else {
                language
            })
        }
        _ => return bad_args(inv),
    };
    core.insert_md_block(view, kind);
    CommandOutcome::Done
}
