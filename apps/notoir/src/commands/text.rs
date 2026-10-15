//! Line operations, find/replace, Go to Line, Command Palette and related
//! commands (docs/specs/11, 13, 15, 16). Editing commands act on the active
//! view (the palette runs them with the context captured when it opened).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::document::ReadOnly;
use notoir_core::ids::ViewId;
use notoir_core::keymap::KeyBindingDef;
use notoir_core::settings::SettingValue;
use notoir_editor::edit_ops::EditResult;
use notoir_editor::line_ops::{self, Case};
use notoir_editor::{Rope, Selection};

use super::Specs;
use crate::core::{AppCore, Effect};
use crate::shell::overlays::PickerKind;

pub(super) fn specs(out: &mut Specs) {
    let k = KeyBindingDef::new;
    let edit = "hasActiveEditor && !readOnly";
    let editor = "hasActiveEditor";
    let line_op = |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, Category::Edit, f)
            .when(edit)
            .expect("valid")
    };
    out.push(line_op("edit.duplicateLine", duplicate_line).keys([k("Mod+Shift+D")]));
    out.push(line_op("edit.deleteLine", delete_line).keys([k("Mod+Shift+K")]));
    out.push(line_op("edit.moveLineUp", move_up).keys([k("Alt+Up")]));
    out.push(line_op("edit.moveLineDown", move_down).keys([k("Alt+Down")]));
    out.push(line_op("edit.sortLines", sort_asc));
    out.push(line_op("edit.sortLinesDescending", sort_desc));
    out.push(line_op("edit.trimTrailingWhitespace", trim));
    out.push(line_op("edit.transformToUppercase", upper));
    out.push(line_op("edit.transformToLowercase", lower));
    out.push(line_op("edit.transformToTitleCase", title));
    out.push(line_op("edit.insertLineAfter", insert_after).keys([k("Mod+Enter")]));
    out.push(line_op("edit.insertLineBefore", insert_before).keys([k("Mod+Shift+Enter")]));
    out.push(line_op("edit.joinLines", join));
    out.push(
        CommandSpec::new("edit.toggleReadOnly", Category::Edit, toggle_read_only)
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("edit.find", Category::Edit, find)
            .keys([k("Mod+F")])
            .when(editor)
            .expect("valid")
            .icon("search"),
    );
    out.push(
        CommandSpec::new("edit.replace", Category::Edit, replace)
            .keys([k("Mod+H").mac("Mod+Alt+F")])
            .when(edit)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("edit.findNext", Category::Edit, find_next)
            .keys([k("F3").mac("Mod+G")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("edit.findPrevious", Category::Edit, find_previous)
            .keys([k("Shift+F3").mac("Mod+Shift+G")])
            .when(editor)
            .expect("valid"),
    );
    out.push(CommandSpec::new(
        "edit.toggleFindCaseSensitive",
        Category::Edit,
        toggle_case,
    ));
    out.push(CommandSpec::new(
        "edit.toggleFindWholeWord",
        Category::Edit,
        toggle_word,
    ));
    out.push(CommandSpec::new(
        "edit.toggleFindRegex",
        Category::Edit,
        toggle_regex,
    ));
    out.push(
        CommandSpec::new(
            "edit.toggleFindInSelection",
            Category::Edit,
            toggle_in_selection,
        )
        .when(editor)
        .expect("valid"),
    );
    out.push(
        CommandSpec::new("edit.replaceOne", Category::Edit, replace_one)
            .when(edit)
            .expect("valid")
            .hidden(),
    );
    out.push(
        CommandSpec::new("edit.replaceAll", Category::Edit, replace_all)
            .when(edit)
            .expect("valid")
            .hidden(),
    );
    out.push(CommandSpec::new("edit.closeFind", Category::Edit, close_find).hidden());
    out.push(
        CommandSpec::new("navigate.goToLine", Category::Navigate, go_to_line)
            .keys([k("Ctrl+G")])
            .when(editor)
            .expect("valid"),
    );
    out.push(
        CommandSpec::new("workbench.commandPalette", Category::View, palette)
            .keys([k("Mod+Shift+P"), k("F1")]),
    );
    out.push(CommandSpec::new(
        "view.toggleMinimap",
        Category::View,
        toggle_minimap,
    ));
    out.push(CommandSpec::new("view.minimapSide", Category::View, minimap_side).hidden());
    out.push(CommandSpec::new("view.minimapScale", Category::View, minimap_scale).hidden());
    out.push(CommandSpec::new(
        "preferences.openKeybindingsFile",
        Category::Preferences,
        open_keybindings,
    ));
    out.push(CommandSpec::new(
        "preferences.openSettingsFile",
        Category::Preferences,
        open_settings,
    ));
}

fn active(core: &AppCore) -> Option<ViewId> {
    core.workbench.active_editor()
}

fn line_op(
    core: &mut AppCore,
    f: impl Fn(&Rope, &Selection) -> Option<EditResult>,
) -> CommandOutcome {
    let Some(view) = active(core) else {
        return CommandOutcome::Done;
    };
    let Some(v) = core.workbench.view(view) else {
        return CommandOutcome::Done;
    };
    let Some(doc) = core.workbench.document(v.document_id) else {
        return CommandOutcome::Done;
    };
    if doc.is_read_only() {
        return CommandOutcome::Done;
    }
    if let Some(r) = f(doc.buffer.rope(), &v.selection) {
        let now = core.now_ms;
        let _ = core
            .workbench
            .edit(view, r.changes, r.selection, r.kind, now);
    }
    CommandOutcome::Done
}

fn duplicate_line(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, line_ops::duplicate_lines)
}
fn delete_line(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, line_ops::delete_lines)
}
fn move_up(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::move_lines(r, s, false))
}
fn move_down(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::move_lines(r, s, true))
}
fn sort_asc(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::sort_lines(r, s, false))
}
fn sort_desc(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::sort_lines(r, s, true))
}
fn trim(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, line_ops::trim_trailing_whitespace)
}
fn upper(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::transform_case(r, s, Case::Upper))
}
fn lower(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::transform_case(r, s, Case::Lower))
}
fn title(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::transform_case(r, s, Case::Title))
}
fn insert_after(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::insert_line(r, s, true))
}
fn insert_before(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, |r, s| line_ops::insert_line(r, s, false))
}
fn join(c: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    line_op(c, line_ops::join_lines)
}

/// Per-document user read-only (lock icon on the tab).
fn toggle_read_only(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(doc) = active(core)
        .and_then(|v| core.workbench.view(v))
        .map(|v| v.document_id)
        .and_then(|d| core.workbench.document_mut(d))
    {
        doc.read_only = match doc.read_only {
            ReadOnly::No => ReadOnly::User,
            // A protected file can be unlocked for editing; saving it then
            // offers Save as Administrator… (spec 36).
            ReadOnly::User | ReadOnly::FileSystem => ReadOnly::No,
            other => other,
        };
    }
    CommandOutcome::Done
}

fn find(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.open_find(v, false);
    }
    CommandOutcome::Done
}

fn replace(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.open_find(v, true);
    }
    CommandOutcome::Done
}

fn find_next(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.find_next(v, true);
    }
    CommandOutcome::Done
}

fn find_previous(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.find_next(v, false);
    }
    CommandOutcome::Done
}

fn refresh_active(core: &mut AppCore) {
    if let Some(v) = active(core) {
        core.refresh_find(v, false);
    }
}

fn toggle_case(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.find.options.case_sensitive ^= true;
    refresh_active(core);
    CommandOutcome::Done
}

fn toggle_word(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.find.options.whole_word ^= true;
    refresh_active(core);
    CommandOutcome::Done
}

fn toggle_regex(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.find.options.regex ^= true;
    refresh_active(core);
    CommandOutcome::Done
}

fn toggle_in_selection(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.toggle_find_in_selection(v);
    }
    CommandOutcome::Done
}

fn replace_one(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.replace_one(v);
    }
    CommandOutcome::Done
}

fn replace_all(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.replace_all(v);
    }
    CommandOutcome::Done
}

fn close_find(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.close_find(v);
    }
    CommandOutcome::Done
}

fn go_to_line(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = active(core) {
        core.open_goto_line(v);
    }
    CommandOutcome::Done
}

fn palette(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    core.effects.push(Effect::OpenPicker(PickerKind::Commands));
    CommandOutcome::Done
}

fn toggle_minimap(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let next = !core.settings.bool("editor.minimap.enabled", None);
    core.set_user_setting("editor.minimap.enabled", Some(SettingValue::Bool(next)));
    CommandOutcome::Done
}

fn minimap_side(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match inv.args.get("side").and_then(|v| v.as_str()) {
        Some(side @ ("left" | "right")) => {
            core.set_user_setting("editor.minimap.side", Some(SettingValue::Str(side.into())));
            CommandOutcome::Done
        }
        _ => super::bad_args(inv),
    }
}

fn minimap_scale(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    match inv.args.get("scale").and_then(|v| v.as_i64()) {
        Some(s @ 1..=3) => {
            core.set_user_setting("editor.minimap.scale", Some(SettingValue::Int(s)));
            CommandOutcome::Done
        }
        _ => super::bad_args(inv),
    }
}

/// Opens `keybindings.toml` (created with a commented template if missing);
/// saving it applies the bindings.
fn open_keybindings(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let path = core.paths.keybindings_file();
    core.open_config_file(path, KEYBINDINGS_TEMPLATE);
    CommandOutcome::Done
}

fn open_settings(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    let path = core.paths.user_settings_file();
    core.open_config_file(path, SETTINGS_TEMPLATE);
    CommandOutcome::Done
}

const KEYBINDINGS_TEMPLATE: &str = "# Notoir keyboard shortcuts. Saved changes apply immediately.\n\
# Each entry adds a binding; prefix the command with '-' to remove a default.\n\
#\n\
# [[binding]]\n\
# key = \"Mod+Shift+S\"\n\
# command = \"file.saveAll\"\n\
#\n\
# [[binding]]\n\
# key = \"Mod+Shift+K\"\n\
# command = \"-edit.deleteLine\"\n";

const SETTINGS_TEMPLATE: &str = "# Notoir user settings. Saved changes apply immediately.\n\
#\n\
# [editor]\n\
# fontSize = 14\n";
