//! Selection menu (docs/specs/11 §Cursor and Selection): multi-cursor,
//! occurrences, column mode and expand/shrink selection.

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::ViewId;
use notoir_core::keymap::KeyBindingDef;
use notoir_editor::multi_cursor;
use notoir_editor::{Rope, Selection};

use super::Specs;
use crate::core::AppCore;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    let spec = |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, Category::Selection, f)
            .when("hasActiveEditor")
            .expect("valid")
    };
    out.push(
        spec("selection.addCursorAbove", |c, _| {
            apply(c, |rope, sel, tab| {
                Some(multi_cursor::add_cursor_vertical(rope, sel, true, tab))
            })
        })
        .keys([k("Mod+Alt+Up").when("editorFocus")]),
    );
    out.push(
        spec("selection.addCursorBelow", |c, _| {
            apply(c, |rope, sel, tab| {
                Some(multi_cursor::add_cursor_vertical(rope, sel, false, tab))
            })
        })
        .keys([k("Mod+Alt+Down").when("editorFocus")]),
    );
    out.push(
        spec("selection.selectNextOccurrence", |c, _| {
            apply(c, |rope, sel, _| {
                multi_cursor::select_next_occurrence(rope, sel)
            })
        })
        .keys([k("Mod+D").when("editorFocus")]),
    );
    out.push(
        spec("selection.selectAllOccurrences", |c, _| {
            apply(c, |rope, sel, _| {
                multi_cursor::select_all_occurrences(rope, sel)
            })
        })
        .keys([k("Mod+Shift+L").when("editorFocus")]),
    );
    out.push(
        spec("selection.expand", expand).keys([k("Shift+Alt+Right")
            .mac("Ctrl+Shift+Mod+Right")
            .when("editorFocus")]),
    );
    out.push(
        spec("selection.shrink", shrink).keys([k("Shift+Alt+Left")
            .mac("Ctrl+Shift+Mod+Left")
            .when("editorFocus")]),
    );
    out.push(CommandSpec::new(
        "selection.columnMode",
        Category::Selection,
        |c, _| {
            c.column_mode = !c.column_mode;
            CommandOutcome::Done
        },
    ));
}

fn target(core: &AppCore) -> Option<(ViewId, Rope, Selection, usize)> {
    let view = core.workbench.active_editor()?;
    let v = core.workbench.view(view)?;
    let doc = core.workbench.document(v.document_id)?;
    Some((
        view,
        doc.buffer.rope().clone(),
        v.selection.clone(),
        core.indent_style(doc.id).tab_size,
    ))
}

fn apply(
    core: &mut AppCore,
    f: impl FnOnce(&Rope, &Selection, usize) -> Option<Selection>,
) -> CommandOutcome {
    if let Some((view, rope, sel, tab)) = target(core)
        && let Some(next) = f(&rope, &sel, tab)
    {
        let _ = core.workbench.set_selection(view, next);
        core.expand_history.remove(&view);
    }
    CommandOutcome::Done
}

fn expand(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some((view, rope, sel, _)) = target(core) {
        let next = multi_cursor::expand_selection(&rope, &sel);
        if next != sel {
            let history = core.expand_history.entry(view).or_default();
            // A selection changed elsewhere starts a new history.
            if history.last().is_some_and(|(_, after)| *after != sel) {
                history.clear();
            }
            history.push((sel, next.clone()));
            let _ = core.workbench.set_selection(view, next);
        }
    }
    CommandOutcome::Done
}

fn shrink(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some((view, _, sel, _)) = target(core) {
        let history = core.expand_history.entry(view).or_default();
        match history.last() {
            Some((before, after)) if *after == sel => {
                let before = before.clone();
                history.pop();
                let _ = core.workbench.set_selection(view, before);
            }
            _ => history.clear(),
        }
    }
    CommandOutcome::Done
}
