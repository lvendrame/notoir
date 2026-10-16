//! Notes commands (docs/specs/34): follow wiki links, link-target and tag
//! pickers, `[[` completion, Backlinks and Tags panels.

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::ViewId;
use notoir_core::keymap::KeyBindingDef;
use notoir_core::panels::PanelId;

use super::{Specs, arg, bad_args};
use crate::core::AppCore;

type Handler = fn(&mut AppCore, &CommandInvocation) -> CommandOutcome;

fn done() -> CommandOutcome {
    CommandOutcome::Done
}

fn show(core: &mut AppCore, panel: PanelId) -> CommandOutcome {
    core.exit_maximize();
    core.panels.show(panel);
    done()
}

pub(super) fn specs(out: &mut Specs) {
    let spec = |id: &'static str, f: Handler| CommandSpec::new(id, Category::Markdown, f);
    let hidden =
        |id: &'static str, f: Handler| CommandSpec::new(id, Category::Markdown, f).hidden();
    // Follow the link at the caret (Ctrl/Cmd+click passes `{ view, offset }`).
    out.push(
        spec("notes.followLink", follow)
            .keys([KeyBindingDef::new("Alt+Enter").when("editorFocus && languageId == 'markdown'")])
            .when("hasActiveEditor && languageId == 'markdown'")
            .expect("valid"),
    );
    out.push(spec("notes.showBacklinks", |c, _| {
        show(c, PanelId::Backlinks)
    }));
    out.push(spec("notes.showTags", |c, _| show(c, PanelId::Tags)));
    out.push(spec("notes.rebuildIndex", |c, _| {
        c.notes_rebuild();
        done()
    }));
    out.push(hidden("notes.pick", |c, i| match arg(i, "value") {
        Some(v) => {
            let v = v.to_owned();
            c.notes_pick(&v);
            done()
        }
        None => bad_args(i),
    }));
    out.push(hidden("notes.pickTag", |c, i| match arg(i, "tag") {
        Some(t) => {
            let t = t.to_owned();
            c.notes_pick_tag(&t);
            done()
        }
        None => bad_args(i),
    }));
    out.push(hidden("notes.filterTag", |c, i| {
        c.notes.tag_filter = arg(i, "tag").map(str::to_owned);
        done()
    }));
    out.push(hidden("notes.complete", |c, i| {
        let index = i.args.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        c.notes_complete(index);
        done()
    }));
    out.push(hidden("notes.findUnlinked", |c, i| match arg(i, "note") {
        Some(rel) => {
            let rel = rel.to_owned();
            c.notes_find_unlinked(&rel);
            done()
        }
        None => bad_args(i),
    }));
    out.push(hidden("notes.openLine", |c, i| {
        let line = i.args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        match arg(i, "note") {
            Some(rel) => {
                let rel = rel.to_owned();
                c.notes_open_line(&rel, line);
                done()
            }
            None => bad_args(i),
        }
    }));
}

/// `{ view, offset }` (Ctrl/Cmd+click) or the active editor's caret.
fn follow(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let view = inv
        .args
        .get("view")
        .and_then(|v| v.as_u64())
        .map(ViewId)
        .or_else(|| core.workbench.active_editor());
    let Some(view) = view else {
        return done();
    };
    let offset = inv
        .args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|o| o as usize)
        .or_else(|| {
            core.workbench
                .view(view)
                .map(|v| v.selection.primary().head)
        });
    if let Some(offset) = offset {
        core.notes_follow_at(view, offset);
    }
    done()
}
