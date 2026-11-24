mod common;
use egui_kittest::kittest::{NodeT, Queryable};
use notoir_core::{ids::ViewId, view::ViewKind};
use notoir_editor::{ChangeSet, EditKind, Selection};
use serde_json::{Value, json};

const SOURCE: &str = "# Notes\n\n| Name | State |\n|:---|---:|\n| α | **ready** |\n\nTail\n";
fn open(source: &str) -> (common::TestApp, ViewId, ViewId) {
    open_theme(source, notoir::theme::ThemeId::Midnight)
}
fn open_theme(source: &str, theme: notoir::theme::ThemeId) -> (common::TestApp, ViewId, ViewId) {
    let text = source.to_owned();
    let mut app = common::start(move |o| {
        o.theme_override = Some(theme);
        let p = o.paths.data.join("table.md");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        o.open_paths = vec![(p, None)];
    });
    common::settle(&mut app);
    let source = app.harness.state().core.workbench.active_view().unwrap();
    common::run(
        &mut app,
        "markdown.editTable",
        json!({"view":source.0,"start":9}),
    );
    let sheet = app.harness.state().core.workbench.active_view().unwrap();
    assert_eq!(
        app.harness.state().core.workbench.view(sheet).unwrap().kind,
        ViewKind::MarkdownTable
    );
    (app, source, sheet)
}
fn text(app: &common::TestApp, view: ViewId) -> String {
    let wb = &app.harness.state().core.workbench;
    wb.document(wb.view(view).unwrap().document_id)
        .unwrap()
        .buffer
        .text()
}
fn action(app: &mut common::TestApp, view: ViewId, mut args: Value) {
    args["view"] = json!(view.0);
    common::run(app, "markdown.tableAction", args);
}
fn edit_source(app: &mut common::TestApp, view: ViewId, changes: ChangeSet) {
    let c = &mut app.harness.state_mut().core;
    c.workbench
        .edit(
            view,
            changes,
            Selection::caret(0),
            EditKind::Other,
            c.now_ms,
        )
        .unwrap();
    app.harness.run_steps(3);
}

// spec 24: "Open a linked table worksheet"
#[test]
fn linked_tab_shares_source_save_and_session_state() {
    let (mut app, source, sheet) = open(SOURCE);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(source)
            .unwrap()
            .document_id,
        app.harness
            .state()
            .core
            .workbench
            .view(sheet)
            .unwrap()
            .document_id
    );
    assert!(app.harness.query_by_label("Go to Markdown").is_some());
    common::run(
        &mut app,
        "markdown.editTable",
        json!({"view":source.0,"start":9}),
    );
    assert_eq!(
        app.harness.state().core.workbench.active_view(),
        Some(sheet)
    );
    assert_eq!(app.harness.state().core.workbench.views.iter().count(), 2);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    action(&mut app, sheet, json!({"op":"set","value":"β"}));
    assert_eq!(text(&app, source), SOURCE.replace(" α ", " β "));
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .view(source)
        .unwrap()
        .document_id;
    assert!(
        app.harness
            .state()
            .core
            .workbench
            .document(doc)
            .unwrap()
            .is_dirty()
    );
    common::run(&mut app, "file.save", Value::Null);
    common::settle(&mut app);
    assert!(
        !app.harness
            .state()
            .core
            .workbench
            .document(doc)
            .unwrap()
            .is_dirty()
    );
    assert_eq!(
        app.harness
            .state()
            .core
            .capture_session()
            .groups
            .iter()
            .map(|g| g.tabs.len())
            .sum::<usize>(),
        1
    );
    common::run(&mut app, "file.close", json!({"view":source.0}));
    assert!(app.harness.state().core.workbench.view(sheet).is_some());
    let saved = app.harness.state().core.capture_session();
    assert_eq!(saved.groups.iter().map(|g| g.tabs.len()).sum::<usize>(), 1);
    action(&mut app, sheet, json!({"op":"source"}));
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(app.harness.state().core.workbench.active_view().unwrap())
            .unwrap()
            .kind,
        ViewKind::Text
    );
}

// spec 24: "Synchronize worksheet and Markdown edits"
#[test]
fn source_changes_map_anchors_and_shared_undo() {
    let (mut app, source, sheet) = open(SOURCE);
    edit_source(&mut app, source, ChangeSet::insert(0, "é prefix\n"));
    assert!(app.harness.state().core.worksheets.views[&sheet].available);
    let current = text(&app, source);
    let at = current.chars().collect::<String>().find("ready").unwrap();
    let at = current[..at].chars().count();
    edit_source(&mut app, source, ChangeSet::replace(at, at + 5, "done"));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[1][1]
            .markdown,
        "**done**"
    );
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    action(&mut app, sheet, json!({"op":"set","value":"β"}));
    common::run(&mut app, "edit.undo", Value::Null);
    assert!(text(&app, source).contains("| α | **done** |"));
    common::run(&mut app, "edit.redo", Value::Null);
    assert!(text(&app, source).contains("| β | **done** |"));
}

// spec 24: "Pause and recover an unavailable worksheet"
#[test]
fn invalid_source_deletion_and_undo_do_not_retarget_other_tables() {
    let double = format!("{SOURCE}\n{SOURCE}");
    let (mut app, source, sheet) = open(&double);
    let original = app
        .harness
        .state()
        .core
        .workbench
        .view(sheet)
        .unwrap()
        .markdown_table
        .as_ref()
        .unwrap()
        .range;
    let separator = double.find(":---").unwrap();
    edit_source(
        &mut app,
        source,
        ChangeSet::replace(separator, separator + 4, "oops"),
    );
    assert!(!app.harness.state().core.worksheets.views[&sheet].available);
    let before = text(&app, source);
    action(
        &mut app,
        sheet,
        json!({"op":"set","value":"must not write"}),
    );
    assert_eq!(text(&app, source), before);
    edit_source(
        &mut app,
        source,
        ChangeSet::replace(separator, separator + 4, ":---"),
    );
    assert!(app.harness.state().core.worksheets.views[&sheet].available);
    edit_source(&mut app, source, ChangeSet::delete(original.0, original.1));
    assert!(!app.harness.state().core.worksheets.views[&sheet].available);
    common::run(&mut app, "edit.undo", Value::Null);
    assert!(app.harness.state().core.worksheets.views[&sheet].available);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(sheet)
            .unwrap()
            .markdown_table
            .as_ref()
            .unwrap()
            .range,
        original
    );
}

// spec 24: "Edit worksheet structure and paste cell ranges"
#[test]
fn rectangular_paste_formatting_and_structure_are_atomic() {
    let (mut app, _source, sheet) = open(SOURCE);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":1}));
    action(
        &mut app,
        sheet,
        json!({"op":"paste","value":"one\ttwo\nthree\tfour\n"}),
    );
    let state = &app.harness.state().core.worksheets.views[&sheet];
    let table = state.table.as_ref().unwrap();
    assert_eq!((table.rows.len(), table.aligns.len()), (3, 3));
    assert_eq!(table.rows[2][2].markdown, "four");
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, sheet), SOURCE);
    common::run(&mut app, "edit.redo", Value::Null);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":1}));
    action(
        &mut app,
        sheet,
        json!({"op":"select","row":2,"column":2,"extend":true}),
    );
    action(&mut app, sheet, json!({"op":"format","value":"italic"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[2][2]
            .markdown,
        "*four*"
    );
    action(&mut app, sheet, json!({"op":"align","value":"center"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .aligns[1],
        notoir_editor::markdown::Align::Center
    );
    action(&mut app, sheet, json!({"op":"select","row":0,"column":0}));
    action(&mut app, sheet, json!({"op":"deleteRows"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows
            .len(),
        3
    );
    action(&mut app, sheet, json!({"op":"deleteColumns"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .aligns
            .len(),
        2
    );
}

#[test]
fn rejects_stale_and_read_only_edits_and_preserves_tables_in_single_editor() {
    let (mut app, source, sheet) = open(SOURCE);
    let before = text(&app, sheet);
    action(
        &mut app,
        sheet,
        json!({"op":"set","value":"stale","revision":999}),
    );
    assert_eq!(text(&app, sheet), before);
    common::run(&mut app, "view.duplicate", json!({"view":sheet.0}));
    let duplicate = app.harness.state().core.workbench.active_view().unwrap();
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(duplicate)
            .unwrap()
            .kind,
        ViewKind::MarkdownTable
    );
    app.harness.state_mut().core.workbench.single_editor();
    assert!(app.harness.state().core.workbench.view(source).is_some());
    assert!(
        app.harness
            .state()
            .core
            .workbench
            .views
            .iter()
            .any(|v| v.kind == ViewKind::MarkdownTable)
    );
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .view(source)
        .unwrap()
        .document_id;
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .read_only = notoir_core::document::ReadOnly::User;
    let sheet = app
        .harness
        .state()
        .core
        .workbench
        .views
        .iter()
        .find(|v| v.kind == ViewKind::MarkdownTable)
        .unwrap()
        .id;
    action(&mut app, sheet, json!({"op":"set","value":"blocked"}));
    assert_eq!(text(&app, sheet), before);
}

#[test]
fn preview_icon_keyboard_live_typing_and_formatting() {
    let (mut app, source, sheet) = open(SOURCE);
    common::run(&mut app, "view.activate", json!({"view":source.0}));
    common::run(&mut app, "markdown.showPreview", Value::Null);
    app.harness
        .query_all_by_label("Edit table as worksheet")
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
        .unwrap()
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.workbench.active_view(),
        Some(sheet)
    );
    app.harness.get_by_label("Row 2, column 1").click();
    app.harness.run_steps(2);
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert!(app.harness.state().core.worksheets.views[&sheet].editing);
    app.harness.event(egui::Event::Text("z".into()));
    app.harness.run_steps(3);
    assert!(text(&app, source).contains("αz"), "{}", text(&app, source));
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    app.harness.key_press_modifiers(ctrl, egui::Key::Z);
    app.harness.run_steps(3);
    assert_eq!(text(&app, source), SOURCE);
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    app.harness.key_press(egui::Key::ArrowRight);
    app.harness.run_steps(2);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].selected,
        (1, 1)
    );
    app.harness.key_press_modifiers(ctrl, egui::Key::I);
    app.harness.run_steps(3);
    assert!(
        text(&app, source).contains("***ready***"),
        "{}",
        text(&app, source)
    );
    app.harness.event(egui::Event::Paste("x\ty\nz\tq".into()));
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[2][2]
            .markdown,
        "q"
    );
}

#[test]
fn worksheet_visual_references() {
    use notoir::theme::ThemeId;
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots");
    std::fs::create_dir_all(&target).unwrap();
    for (theme, name) in [
        (ThemeId::Midnight, "midnight"),
        (ThemeId::Paper, "paper"),
        (ThemeId::Dracula, "dracula"),
    ] {
        let (mut app, source, sheet) = open_theme(SOURCE, theme);
        common::run(&mut app, "view.activate", json!({"view":source.0}));
        common::run(&mut app, "markdown.showPreview", Value::Null);
        app.harness.get_by_label("Edit table as worksheet").hover();
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-table-hover-{name}.png")))
            .unwrap();
        app.harness
            .query_all_by_label("Edit table as worksheet")
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
            .unwrap()
            .click();
        app.harness.run_steps(3);
        action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
        action(
            &mut app,
            sheet,
            json!({"op":"select","row":1,"column":1,"extend":true}),
        );
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-{name}.png")))
            .unwrap();
        action(&mut app, sheet, json!({"op":"edit","row":0,"column":0}));
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-header-edit-{name}.png")))
            .unwrap();
        action(&mut app, sheet, json!({"op":"edit","row":1,"column":1}));
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-edit-{name}.png")))
            .unwrap();
        app.harness.set_size(egui::vec2(620.0, 600.0));
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-narrow-{name}.png")))
            .unwrap();
        app.harness.set_size(egui::vec2(1280.0, 800.0));
        app.harness.run_steps(3);
        action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
        common::run(&mut app, "markdown.insertLink", json!({"view":sheet.0}));
        app.harness
            .state_mut()
            .core
            .markdown
            .insert_dialog
            .as_mut()
            .unwrap()
            .destination = "https://example.com".into();
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-link-{name}.png")))
            .unwrap();
        app.harness.key_press(egui::Key::Escape);
        app.harness.run_steps(3);
        let range = app
            .harness
            .state()
            .core
            .workbench
            .view(sheet)
            .unwrap()
            .markdown_table
            .as_ref()
            .unwrap()
            .range;
        edit_source(&mut app, source, ChangeSet::delete(range.0, range.1));
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-unavailable-{name}.png")))
            .unwrap();
    }
}

#[test]
fn typing_coalesces_and_reload_updates_the_original_table() {
    let (mut app, source, sheet) = open(SOURCE);
    action(&mut app, sheet, json!({"op":"edit","row":1,"column":0}));
    action(&mut app, sheet, json!({"op":"set","value":"αx"}));
    action(&mut app, sheet, json!({"op":"set","value":"αxy"}));
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), SOURCE);
    let reload = format!("New prefix\n{SOURCE}").replace("ready", "updated");
    let core = &mut app.harness.state_mut().core;
    let doc = core.workbench.view(source).unwrap().document_id;
    core.workbench
        .apply_edit(
            doc,
            None,
            ChangeSet::replace(0, SOURCE.chars().count(), reload.clone()),
            Selection::caret(0),
            EditKind::Other,
            notoir_core::events::ChangeOrigin::Reload,
            core.now_ms,
        )
        .unwrap();
    app.harness.run_steps(3);
    let state = &app.harness.state().core.worksheets.views[&sheet];
    assert!(state.available);
    assert_eq!(
        state.table.as_ref().unwrap().rows[1][1].markdown,
        "**updated**"
    );
}

#[test]
fn source_can_append_rows_at_the_tracked_table_boundary() {
    let (mut app, source, sheet) = open(SOURCE);
    let end = app
        .harness
        .state()
        .core
        .workbench
        .view(sheet)
        .unwrap()
        .markdown_table
        .as_ref()
        .unwrap()
        .range
        .1;
    edit_source(&mut app, source, ChangeSet::insert(end, "| new | row |\n"));
    let state = &app.harness.state().core.worksheets.views[&sheet];
    assert!(state.available);
    assert_eq!(state.table.as_ref().unwrap().rows.len(), 3);
    action(&mut app, sheet, json!({"op":"select","row":2,"column":0}));
    action(&mut app, sheet, json!({"op":"set","value":"updated"}));
    assert!(text(&app, source).contains("| updated | row |"));
}

#[test]
fn paste_growth_preserves_surplus_source_cells() {
    let source = SOURCE.replace("| α | **ready** |", "| α | **ready** | hidden |");
    let (mut app, _, sheet) = open(&source);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":1}));
    action(&mut app, sheet, json!({"op":"paste","value":"one\ttwo"}));
    assert!(text(&app, sheet).contains("| α | one | two | hidden |"));
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, sheet), source);
}

#[test]
fn borderless_header_edits_keep_the_table_binding() {
    let source = "# Notes\n\nName | State\n---|---\nα | ready\n\nTail\n";
    let (mut app, original, sheet) = open(source);
    edit_source(&mut app, original, ChangeSet::insert(9, "Long "));
    assert!(app.harness.state().core.worksheets.views[&sheet].available);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[0][0]
            .markdown,
        "Long Name"
    );
    action(&mut app, sheet, json!({"op":"select","row":0,"column":0}));
    action(&mut app, sheet, json!({"op":"set","value":"Other header"}));
    assert!(app.harness.state().core.worksheets.views[&sheet].available);
    assert!(text(&app, sheet).contains("Other header | State"));
}

#[test]
fn changing_language_pauses_and_resumes_the_worksheet_without_a_text_edit() {
    let (mut app, source, sheet) = open(SOURCE);
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .view(source)
        .unwrap()
        .document_id;
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .language = notoir_editor::LanguageId::PLAINTEXT;
    action(&mut app, sheet, json!({"op":"set","value":"blocked"}));
    assert_eq!(text(&app, source), SOURCE);
    assert!(!app.harness.state().core.worksheets.views[&sheet].available);
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .language = notoir_editor::LanguageId::MARKDOWN;
    app.harness.run_steps(3);
    assert!(app.harness.state().core.worksheets.views[&sheet].available);
}

#[test]
fn active_cell_formats_and_unformats_selected_words_and_links() {
    fn direct_action(app: &mut common::TestApp, view: ViewId, mut args: Value) {
        args["view"] = json!(view.0);
        let inv = notoir_core::command::CommandInvocation::new(
            "markdown.tableAction",
            notoir_core::command::InvocationSource::Internal,
        )
        .with_args(args);
        app.harness.state_mut().core.worksheet_action(&inv);
        app.harness.run_steps(3);
    }
    let (mut app, _, sheet) = open(SOURCE);
    direct_action(&mut app, sheet, json!({"op":"edit","row":1,"column":1}));
    app.harness
        .state_mut()
        .core
        .worksheets
        .views
        .get_mut(&sheet)
        .unwrap()
        .text_selection = Some(notoir_editor::Range::new(2, 7));
    direct_action(&mut app, sheet, json!({"op":"format","value":"bold"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[1][1]
            .markdown,
        "ready"
    );
    direct_action(&mut app, sheet, json!({"op":"set","value":"tick ` here"}));
    app.harness
        .state_mut()
        .core
        .worksheets
        .views
        .get_mut(&sheet)
        .unwrap()
        .text_selection = Some(notoir_editor::Range::new(0, 11));
    direct_action(&mut app, sheet, json!({"op":"format","value":"code"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[1][1]
            .markdown,
        "``tick ` here``"
    );
    direct_action(&mut app, sheet, json!({"op":"set","value":"caption"}));
    app.harness
        .state_mut()
        .core
        .worksheets
        .views
        .get_mut(&sheet)
        .unwrap()
        .text_selection = Some(notoir_editor::Range::new(0, 7));
    app.harness.state_mut().core.insert_md_link(sheet);
    app.harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap()
        .destination = "https://example.com/a b".into();
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[1][1]
            .markdown,
        "[caption](https://example.com/a%20b)"
    );
}

#[test]
fn worksheet_link_icon_uses_the_shared_dialog_and_safe_undoable_insertion() {
    use egui::accesskit::Role;
    let (mut app, source, sheet) = open(SOURCE);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    app.harness
        .get_by_role_and_label(Role::Button, "Link…")
        .click();
    app.harness.run_steps(3);
    let dialog = app
        .harness
        .state()
        .core
        .markdown
        .insert_dialog
        .as_ref()
        .unwrap();
    assert_eq!(dialog.kind, notoir::markdown_insert::InsertDialogKind::Link);
    assert_eq!(dialog.caption, "α");
    assert!(dialog.worksheet.is_some());
    assert!(
        app.harness
            .query_by_role_and_label(Role::TextInput, "Caption")
            .is_some()
    );
    assert!(
        app.harness
            .query_by_role_and_label(Role::TextInput, "URL or file")
            .is_some()
    );
    assert_eq!(text(&app, source), SOURCE);
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(3);
    assert!(app.harness.state().core.markdown.insert_dialog.is_none());
    assert_eq!(text(&app, source), SOURCE);
    common::run(&mut app, "markdown.insertLink", json!({"view":sheet.0}));
    let dialog = app
        .harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap();
    dialog.caption = "a | b".into();
    dialog.destination = "https://example.com/a b".into();
    app.harness.run_steps(2);
    app.harness
        .get_by_role_and_label(Role::Button, "Insert")
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        text(&app, source),
        SOURCE.replace(" α ", " [a \\| b](https://example.com/a%20b) ")
    );
    assert!(app.harness.state().core.markdown.insert_dialog.is_none());
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), SOURCE);
}

#[test]
fn worksheet_link_dialog_rejects_a_changed_source() {
    let (mut app, source, sheet) = open(SOURCE);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    common::run(&mut app, "markdown.insertLink", Value::Null);
    app.harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap()
        .destination = "https://example.com".into();
    edit_source(&mut app, source, ChangeSet::insert(0, "changed\n"));
    let before = text(&app, source);
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(text(&app, source), before);
    assert!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .error
    );
}

#[test]
fn worksheet_link_dialog_completes_workspace_files() {
    use egui::{Event, Key, accesskit::Role};
    let (mut app, source, sheet) = open(SOURCE);
    let root = {
        let core = &app.harness.state().core;
        core.workbench
            .document(core.workbench.view(source).unwrap().document_id)
            .unwrap()
            .path()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned()
    };
    std::fs::write(root.join("target #1.md"), "# Target").unwrap();
    app.harness.state_mut().core.open_workspace(&root);
    for _ in 0..100 {
        if app
            .harness
            .state()
            .core
            .workspace
            .as_ref()
            .is_some_and(|w| w.index_ready)
        {
            break;
        }
        common::settle(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    common::run(&mut app, "markdown.insertLink", json!({"view":sheet.0}));
    app.harness
        .get_by_role_and_label(Role::TextInput, "URL or file")
        .click();
    app.harness.event(Event::Text("@target".into()));
    app.harness.run_steps(3);
    assert!(
        app.harness
            .query_all_by_label("target #1.md")
            .next()
            .is_some()
    );
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness
            .state()
            .core
            .markdown
            .insert_dialog
            .as_ref()
            .unwrap()
            .destination,
        "target %231.md"
    );
    assert_eq!(text(&app, source), SOURCE);
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(
        text(&app, source),
        SOURCE.replace(" α ", " [α](target%20%231.md) ")
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), SOURCE);
}

// spec 24: "Edit an existing worksheet link"
#[test]
fn existing_cell_link_opens_for_editing_and_updates_in_one_undo_step() {
    use egui::accesskit::Role;
    let source = SOURCE.replace(" α ", " [Docs](https://old.example) ");
    let (mut app, original, sheet) = open(&source);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    app.harness
        .get_by_role_and_label(Role::Button, "Link…")
        .click();
    app.harness.run_steps(3);
    let dialog = app
        .harness
        .state()
        .core
        .markdown
        .insert_dialog
        .as_ref()
        .unwrap();
    assert_eq!(dialog.caption, "Docs");
    assert_eq!(dialog.destination, "https://old.example");
    assert_eq!(text(&app, original), source);
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(3);
    assert_eq!(text(&app, original), source);
    common::run(&mut app, "markdown.insertLink", json!({"view":sheet.0}));
    let dialog = app
        .harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap();
    dialog.caption = "Updated docs".into();
    dialog.destination = "https://new.example/path".into();
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(
        text(&app, original),
        source.replace(
            "[Docs](https://old.example)",
            "[Updated docs](https://new.example/path)"
        )
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, original), source);
    common::run(&mut app, "edit.redo", Value::Null);
    assert!(text(&app, original).contains("[Updated docs](https://new.example/path)"));
}

#[test]
fn editing_a_link_from_an_active_cell_preserves_surrounding_text_and_styling() {
    let source = SOURCE.replace(
        " α ",
        " before [**café**](https://old.example \"Docs\") after ",
    );
    let (mut app, original, sheet) = open(&source);
    action(&mut app, sheet, json!({"op":"edit","row":1,"column":0}));
    let core = &mut app.harness.state_mut().core;
    core.worksheets
        .views
        .get_mut(&sheet)
        .unwrap()
        .text_selection = Some(notoir_editor::Range::caret(12));
    core.insert_md_link(sheet);
    let dialog = core.markdown.insert_dialog.as_mut().unwrap();
    assert_eq!(dialog.caption, "café");
    assert_eq!(dialog.destination, "https://old.example");
    dialog.destination = "https://new.example".into();
    core.confirm_md_insert(None);
    app.harness.run_steps(3);
    assert_eq!(
        text(&app, original),
        source.replace("https://old.example", "https://new.example")
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, original), source);
}

#[test]
fn existing_reference_link_resolves_its_destination_for_editing() {
    let source = format!(
        "{}\n[docs]: https://old.example \"Docs\"\n",
        SOURCE.replace(" α ", " [Guide][docs] ")
    );
    let (mut app, original, sheet) = open(&source);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    common::run(&mut app, "markdown.insertLink", Value::Null);
    let dialog = app
        .harness
        .state_mut()
        .core
        .markdown
        .insert_dialog
        .as_mut()
        .unwrap();
    assert_eq!(dialog.caption, "Guide");
    assert_eq!(dialog.destination, "https://old.example");
    dialog.destination = "https://new.example".into();
    common::run(&mut app, "markdown.confirmInsert", Value::Null);
    assert_eq!(
        text(&app, original),
        source.replace("[Guide][docs]", "[Guide](https://new.example \"Docs\")")
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, original), source);
}

const ROWS: &str = "# Notes\n\n| Name | State |\n|:---|---:|\n| a | 1 |\n| b | 2 |\n| c | 3 |\n| d | 4 |\n\nTail\n";
fn row_values(app: &common::TestApp, sheet: ViewId) -> Vec<String> {
    app.harness.state().core.worksheets.views[&sheet]
        .table
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .map(|r| r[0].markdown.clone())
        .collect()
}
fn ctrl() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}
fn select_row_ui(app: &mut common::TestApp, row: usize, mods: egui::Modifiers) {
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &format!("Select row {row}"))
        .click_modifiers(mods);
    app.harness.run_steps(3);
}
fn row_context(app: &mut common::TestApp, row: usize) {
    app.harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &format!("Select row {row}"))
        .click_secondary();
    app.harness.run_steps(3);
}
fn drag_rows(app: &mut common::TestApp, from: egui::Pos2, to: egui::Pos2) {
    app.harness.hover_at(from);
    app.harness.run_steps(1);
    app.harness.drag_at(from);
    app.harness.run_steps(1);
    for step in 1..=8 {
        app.harness
            .hover_at(from + (to - from) * (step as f32 / 8.0));
        app.harness.run_steps(1);
    }
    app.harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    app.harness.run_steps(3);
}

// spec 24: "Select and delete worksheet rows from their counters"
#[test]
fn row_counters_select_ranges_and_nonadjacent_rows_and_delete_only_the_selection() {
    use egui::{Modifiers, accesskit::Role};
    let (mut app, source, sheet) = open(ROWS);
    select_row_ui(&mut app, 2, Modifiers::NONE);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![1]
    );
    assert!(app.harness.state().core.worksheets.views[&sheet].contains(1, 1));
    assert!(!app.harness.state().core.worksheets.views[&sheet].contains(2, 0));
    select_row_ui(&mut app, 4, ctrl());
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![1, 3]
    );
    select_row_ui(&mut app, 5, Modifiers::SHIFT);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![3, 4]
    );
    select_row_ui(&mut app, 2, ctrl());
    select_row_ui(&mut app, 4, ctrl());
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![1, 4]
    );
    row_context(&mut app, 2);
    for label in [
        "Insert row above",
        "Insert row below",
        "Delete selected rows",
    ] {
        assert!(
            app.harness
                .query_by_role_and_label(Role::Button, label)
                .is_some()
        );
    }
    assert!(
        app.harness
            .query_by_role_and_label(Role::Button, "Insert column before")
            .is_none()
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![1, 4],
        "right-click preserves a selected group"
    );
    app.harness
        .get_by_role_and_label(Role::Button, "Delete selected rows")
        .click();
    app.harness.run_steps(3);
    assert_eq!(row_values(&app, sheet), vec!["Name", "b", "c"]);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![1, 4]
    );
}

// spec 24: "Select worksheet columns and use their context menu"
#[test]
fn column_headers_select_whole_columns_and_offer_column_operations() {
    use egui::{Modifiers, accesskit::Role};
    let (mut app, source, sheet) = open(ROWS);
    app.harness
        .get_by_role_and_label(Role::Button, "Row 1, column 2")
        .click();
    app.harness.run_steps(3);
    let s = &app.harness.state().core.worksheets.views[&sheet];
    assert_eq!(
        s.selection_kind,
        notoir::markdown_table::SelectionKind::Columns
    );
    assert_eq!(s.columns(), vec![1]);
    assert!(s.contains(0, 1) && s.contains(4, 1) && !s.contains(1, 0));
    app.harness
        .get_by_role_and_label(Role::Button, "Row 1, column 1")
        .click_modifiers(ctrl());
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].columns(),
        vec![0, 1]
    );
    app.harness
        .get_by_role_and_label(Role::Button, "Row 1, column 2")
        .click_secondary();
    app.harness.run_steps(3);
    for label in [
        "Insert column before",
        "Insert column after",
        "Delete selected columns",
    ] {
        assert!(
            app.harness
                .query_by_role_and_label(Role::Button, label)
                .is_some()
        );
    }
    assert!(
        app.harness
            .query_by_role_and_label(Role::Button, "Insert row above")
            .is_none()
    );
    assert!(
        app.harness
            .get_by_role_and_label(Role::Button, "Delete selected columns")
            .accesskit_node()
            .is_disabled(),
        "the last column is protected"
    );
    app.harness
        .get_by_role_and_label(Role::Button, "Insert column before")
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .aligns
            .len(),
        3
    );
    assert_eq!(row_values(&app, sheet), vec!["", "", "", "", ""]);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].columns(),
        vec![0]
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    app.harness
        .get_by_role_and_label(Role::Button, "Row 1, column 2")
        .click_modifiers(Modifiers::NONE);
    app.harness.run_steps(3);
    app.harness
        .get_by_role_and_label(Role::Button, "Row 1, column 2")
        .click_secondary();
    app.harness.run_steps(3);
    app.harness
        .get_by_role_and_label(Role::Button, "Insert column after")
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .rows[1]
            .iter()
            .map(|c| c.markdown.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "1", ""]
    );
    common::run(&mut app, "edit.undo", Value::Null);
    app.harness
        .get_by_role_and_label(Role::Button, "Row 1, column 2")
        .click_secondary();
    app.harness.run_steps(3);
    app.harness
        .get_by_role_and_label(Role::Button, "Delete selected columns")
        .click();
    app.harness.run_steps(3);
    assert_eq!(row_values(&app, sheet), vec!["Name", "a", "b", "c", "d"]);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .table
            .as_ref()
            .unwrap()
            .aligns
            .len(),
        1
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
}

#[test]
fn row_context_inserts_above_and_below_the_clicked_row() {
    use egui::accesskit::Role;
    let (mut app, source, sheet) = open(ROWS);
    row_context(&mut app, 3);
    app.harness
        .get_by_role_and_label(Role::Button, "Insert row above")
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        row_values(&app, sheet),
        vec!["Name", "a", "", "b", "c", "d"]
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![2]
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    row_context(&mut app, 3);
    app.harness
        .get_by_role_and_label(Role::Button, "Insert row below")
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        row_values(&app, sheet),
        vec!["Name", "a", "b", "", "c", "d"]
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![3]
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    row_context(&mut app, 1);
    assert!(
        app.harness
            .get_by_role_and_label(Role::Button, "Delete selected rows")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        app.harness
            .get_by_role_and_label(Role::Button, "Insert row above")
            .accesskit_node()
            .is_disabled()
    );
}

// spec 24: "Drag selected worksheet rows together"
#[test]
fn dragging_selected_rows_from_a_cell_moves_the_group_and_undo_restores_its_selection() {
    use egui::{Modifiers, accesskit::Role};
    let (mut app, source, sheet) = open(ROWS);
    select_row_ui(&mut app, 2, Modifiers::NONE);
    select_row_ui(&mut app, 4, ctrl());
    let from = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 2, column 2")
        .rect()
        .center();
    let target = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 5, column 1")
        .rect();
    drag_rows(
        &mut app,
        from,
        egui::pos2(target.center().x, target.bottom() - 1.0),
    );
    assert_eq!(row_values(&app, sheet), vec!["Name", "b", "d", "a", "c"]);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![3, 4]
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![1, 3]
    );
    common::run(&mut app, "edit.redo", Value::Null);
    assert_eq!(row_values(&app, sheet), vec!["Name", "b", "d", "a", "c"]);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].rows(),
        vec![3, 4]
    );
}

#[test]
fn dragging_a_counter_upward_and_cancelling_a_drag() {
    use egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
    let (mut app, source, sheet) = open(ROWS);
    let from = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 4")
        .rect()
        .center();
    let target = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 2")
        .rect();
    drag_rows(
        &mut app,
        from,
        egui::pos2(target.center().x, target.top() + 1.0),
    );
    assert_eq!(row_values(&app, sheet), vec!["Name", "c", "a", "b", "d"]);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    select_row_ui(&mut app, 2, Modifiers::NONE);
    let from = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 2")
        .rect()
        .center();
    let to = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 5")
        .rect()
        .center();
    app.harness.drag_at(from);
    app.harness.run_steps(1);
    app.harness.hover_at(to);
    app.harness.run_steps(2);
    assert!(
        app.harness.state().core.worksheets.views[&sheet]
            .row_drag
            .is_some()
    );
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(1);
    app.harness.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(3);
    assert_eq!(text(&app, source), ROWS);
    app.harness.drag_at(from);
    app.harness.run_steps(1);
    app.harness.hover_at(to);
    app.harness.run_steps(2);
    edit_source(&mut app, source, ChangeSet::insert(0, "changed\n"));
    app.harness.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(3);
    assert_eq!(text(&app, source), format!("changed\n{ROWS}"));
}

#[test]
fn sticky_row_numbers_and_headers_survive_horizontal_and_vertical_scrolling() {
    use egui::accesskit::Role;
    let mut source =
        "# Notes\n\n| A | B | C | D | E | F | G | H |\n|---|---|---|---|---|---|---|---|\n"
            .to_owned();
    for r in 0..80 {
        source.push_str(&format!("| {r} | b | c | d | e | f | g | h |\n"));
    }
    let (mut app, _, sheet) = open(&source);
    app.harness.set_size(egui::vec2(760.0, 420.0));
    app.harness.run_steps(3);
    let original = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 1")
        .rect();
    app.harness
        .state_mut()
        .core
        .worksheets
        .views
        .get_mut(&sheet)
        .unwrap()
        .scroll = (400.0, 400.0);
    app.harness.run_steps(3);
    let header = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 1")
        .rect();
    assert_eq!(header.left_top(), original.left_top());
    let counter = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 16")
        .rect();
    assert_eq!(counter.left(), header.left());
    assert!(counter.top() >= header.bottom());
    assert!(app.harness.state().core.worksheets.views[&sheet].scroll.0 >= 399.0);
    assert!(app.harness.state().core.worksheets.views[&sheet].scroll.1 >= 399.0);
}

#[test]
fn worksheet_row_and_column_visual_references() {
    use egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
    use notoir::theme::ThemeId;
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots");
    std::fs::create_dir_all(&target).unwrap();
    for (theme, name) in [
        (ThemeId::Midnight, "midnight"),
        (ThemeId::Paper, "paper"),
        (ThemeId::Dracula, "dracula"),
    ] {
        let (mut app, source, sheet) = open_theme(ROWS, theme);
        select_row_ui(&mut app, 2, Modifiers::NONE);
        select_row_ui(&mut app, 4, ctrl());
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-rows-{name}.png")))
            .unwrap();
        row_context(&mut app, 2);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-row-menu-{name}.png")))
            .unwrap();
        app.harness.key_press(Key::Escape);
        app.harness.run_steps(3);
        app.harness
            .get_by_role_and_label(Role::Button, "Row 1, column 2")
            .click();
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-column-{name}.png")))
            .unwrap();
        app.harness
            .get_by_role_and_label(Role::Button, "Row 1, column 2")
            .click_secondary();
        app.harness.run_steps(3);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-column-menu-{name}.png")))
            .unwrap();
        app.harness.key_press(Key::Escape);
        app.harness.run_steps(3);
        select_row_ui(&mut app, 2, Modifiers::NONE);
        select_row_ui(&mut app, 4, ctrl());
        let from = app
            .harness
            .get_by_role_and_label(Role::Button, "Row 2, column 2")
            .rect()
            .center();
        let to = app
            .harness
            .get_by_role_and_label(Role::Button, "Row 5, column 1")
            .rect();
        let to = egui::pos2(to.center().x, to.bottom() - 1.0);
        app.harness.drag_at(from);
        app.harness.run_steps(1);
        app.harness.hover_at(from + egui::vec2(0.0, 10.0));
        app.harness.run_steps(1);
        app.harness.hover_at(to);
        app.harness.run_steps(3);
        assert_eq!(
            text(&app, source),
            ROWS,
            "a drag changes source only on drop"
        );
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-row-drag-{name}.png")))
            .unwrap();
        app.harness.event(Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        app.harness.run_steps(3);
        assert_eq!(row_values(&app, sheet), vec!["Name", "b", "d", "a", "c"]);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-row-moved-{name}.png")))
            .unwrap();
    }
}

const LONG_ROWS: &str = "# Notes\n\n| Name | Description |\n|---|---|\n| A name that needs more room than the default column offers | This is a longer description with **important details**, an [existing link](https://example.com), and enough text to wrap over several lines when its column is narrow. Every part of this description should be readable after fitting the row. |\n| b | short |\n\nTail\n";
fn resize_action(app: &mut common::TestApp, sheet: ViewId, row: bool, index: usize, size: f32) {
    action(
        app,
        sheet,
        json!({"op":if row {"resizeRow"}else{"resizeColumn"},"row":index,"column":index,"size":size}),
    );
}
fn fit_handle(app: &mut common::TestApp, label: &str) {
    use egui::{Event, Modifiers, PointerButton, accesskit::Role};
    let pos = app
        .harness
        .get_by_role_and_label(Role::Slider, label)
        .rect()
        .center();
    app.harness.hover_at(pos);
    app.harness.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            app.harness.input_mut().events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    }
    app.harness.run_steps(3);
}

// spec 24: "Resize worksheet rows and columns without changing Markdown"
#[test]
fn column_and_row_handles_resize_the_view_without_editing_the_document() {
    use egui::accesskit::Role;
    let (mut app, source, sheet) = open(LONG_ROWS);
    let from = app
        .harness
        .get_by_role_and_label(Role::Slider, "Resize column 1")
        .rect()
        .center();
    drag_rows(&mut app, from, from + egui::vec2(120.0, 0.0));
    assert!(
        (app.harness.state().core.worksheets.views[&sheet].column_width(0) - 304.0).abs() < 1.0
    );
    let header = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 1, column 1")
        .rect();
    let cell = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 2, column 1")
        .rect();
    assert_eq!(header.width(), cell.width());
    let from = app
        .harness
        .get_by_role_and_label(Role::Slider, "Resize row 2")
        .rect()
        .center();
    drag_rows(&mut app, from, from + egui::vec2(0.0, 110.0));
    assert!((app.harness.state().core.worksheets.views[&sheet].row_height(1) - 140.0).abs() < 1.0);
    assert_eq!(
        app.harness
            .get_by_role_and_label(Role::Button, "Row 2, column 1")
            .rect()
            .height(),
        140.0
    );
    let row3 = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 3")
        .rect();
    let row2 = app
        .harness
        .get_by_role_and_label(Role::Button, "Select row 2")
        .rect();
    assert_eq!(row3.top(), row2.bottom());
    let from = app
        .harness
        .get_by_role_and_label(Role::Slider, "Resize row 1")
        .rect()
        .center();
    drag_rows(&mut app, from, from + egui::vec2(0.0, 45.0));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(0),
        75.0
    );
    let header = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 1, column 1")
        .rect();
    let data = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 2, column 1")
        .rect();
    assert_eq!(header.height(), 75.0);
    assert_eq!(data.top(), header.bottom());
    assert_eq!(text(&app, source), LONG_ROWS);
    let core = &app.harness.state().core;
    let doc = core
        .workbench
        .document(core.workbench.view(source).unwrap().document_id)
        .unwrap();
    assert!(!doc.is_dirty());
    assert!(!doc.buffer.can_undo());
}

// spec 24: "Auto-fit worksheet content"
#[test]
fn double_clicking_handles_fits_wrapped_content_and_column_width() {
    use egui::accesskit::Role;
    let (mut app, source, sheet) = open(LONG_ROWS);
    resize_action(&mut app, sheet, false, 1, 130.0);
    fit_handle(&mut app, "Resize row 2");
    let height = app.harness.state().core.worksheets.views[&sheet].row_height(1);
    assert!(
        height > 150.0,
        "wrapped content should need multiple lines: {height}"
    );
    assert_eq!(
        app.harness
            .get_by_role_and_label(Role::Button, "Row 2, column 2")
            .rect()
            .height(),
        height
    );
    fit_handle(&mut app, "Resize column 1");
    assert!(app.harness.state().core.worksheets.views[&sheet].column_width(0) > 250.0);
    fit_handle(&mut app, "Resize row 3");
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(2),
        30.0
    );
    assert_eq!(text(&app, source), LONG_ROWS);
    let core = &app.harness.state().core;
    assert!(
        !core
            .workbench
            .document(core.workbench.view(source).unwrap().document_id)
            .unwrap()
            .is_dirty()
    );
}

#[test]
fn resizing_can_be_cancelled_and_is_available_for_read_only_documents() {
    use egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
    let (mut app, source, sheet) = open(ROWS);
    let from = app
        .harness
        .get_by_role_and_label(Role::Slider, "Resize column 1")
        .rect()
        .center();
    let to = from + egui::vec2(80.0, 0.0);
    app.harness.drag_at(from);
    app.harness.run_steps(1);
    app.harness.hover_at(to);
    app.harness.run_steps(3);
    assert!(app.harness.state().core.worksheets.views[&sheet].column_width(0) > 250.0);
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    app.harness.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(0),
        184.0
    );
    app.harness
        .get_by_role_and_label(Role::Slider, "Resize column 1")
        .click();
    app.harness.run_steps(2);
    app.harness.key_press(Key::ArrowRight);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(0),
        192.0
    );
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .view(source)
        .unwrap()
        .document_id;
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .read_only = notoir_core::document::ReadOnly::User;
    app.harness.run_steps(3);
    let from = app
        .harness
        .get_by_role_and_label(Role::Slider, "Resize row 2")
        .rect()
        .center();
    drag_rows(&mut app, from, from + egui::vec2(0.0, 50.0));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(1),
        80.0
    );
    assert_eq!(text(&app, source), ROWS);
    assert!(
        !app.harness
            .state()
            .core
            .workbench
            .document(doc)
            .unwrap()
            .is_dirty()
    );
}

#[test]
fn row_sizes_move_with_rows_and_structural_undo_restores_the_layout() {
    let (mut app, source, sheet) = open(ROWS);
    resize_action(&mut app, sheet, true, 1, 90.0);
    resize_action(&mut app, sheet, true, 3, 140.0);
    resize_action(&mut app, sheet, false, 1, 250.0);
    action(&mut app, sheet, json!({"op":"selectRows","row":1}));
    action(
        &mut app,
        sheet,
        json!({"op":"selectRows","row":3,"toggle":true}),
    );
    action(
        &mut app,
        sheet,
        json!({"op":"moveRows","rows":[1,3],"before":5}),
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .layout
            .row_heights
            .as_slice(),
        &[30.0, 30.0, 30.0, 90.0, 140.0]
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .layout
            .row_heights
            .as_slice(),
        &[30.0, 90.0, 30.0, 140.0, 30.0]
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(1),
        250.0
    );
    resize_action(&mut app, sheet, false, 1, 500.0);
    common::run(&mut app, "edit.redo", Value::Null);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(1),
        500.0,
        "reordering rows does not undo a column resize"
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet]
            .layout
            .row_heights
            .as_slice(),
        &[30.0, 30.0, 30.0, 90.0, 140.0]
    );
}

#[test]
fn row_and_column_sizes_follow_insertions_and_persist_through_text_undo() {
    let (mut app, source, sheet) = open(ROWS);
    resize_action(&mut app, sheet, true, 1, 90.0);
    resize_action(&mut app, sheet, false, 1, 250.0);
    action(&mut app, sheet, json!({"op":"selectRows","row":1}));
    action(&mut app, sheet, json!({"op":"rowBefore"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(1),
        30.0
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(2),
        90.0
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(1),
        90.0
    );
    action(&mut app, sheet, json!({"op":"selectColumns","column":1}));
    action(&mut app, sheet, json!({"op":"columnBefore"}));
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(1),
        184.0
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(2),
        250.0
    );
    common::run(&mut app, "edit.undo", Value::Null);
    action(&mut app, sheet, json!({"op":"select","row":1,"column":0}));
    action(&mut app, sheet, json!({"op":"set","value":"new"}));
    resize_action(&mut app, sheet, true, 1, 110.0);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app, source), ROWS);
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].row_height(1),
        110.0,
        "undoing text does not undo a view-only resize"
    );
    assert_eq!(
        app.harness.state().core.worksheets.views[&sheet].column_width(1),
        250.0
    );
}

#[test]
fn worksheet_resized_content_visual_references() {
    use notoir::theme::ThemeId;
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots");
    std::fs::create_dir_all(&target).unwrap();
    for (theme, name) in [
        (ThemeId::Midnight, "midnight"),
        (ThemeId::Paper, "paper"),
        (ThemeId::Dracula, "dracula"),
    ] {
        let (mut app, source, sheet) = open_theme(LONG_ROWS, theme);
        resize_action(&mut app, sheet, false, 0, 240.0);
        resize_action(&mut app, sheet, false, 1, 350.0);
        resize_action(&mut app, sheet, true, 1, 170.0);
        app.harness.hover_at(egui::pos2(1100.0, 700.0));
        app.harness.run_steps(2);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-resized-{name}.png")))
            .unwrap();
        fit_handle(&mut app, "Resize row 2");
        app.harness.hover_at(egui::pos2(1100.0, 700.0));
        app.harness.run_steps(2);
        app.harness
            .render()
            .unwrap()
            .save(target.join(format!("markdown-worksheet-auto-fit-{name}.png")))
            .unwrap();
        assert_eq!(text(&app, source), LONG_ROWS);
    }
}

#[test]
fn double_clicking_a_header_keeps_the_caption_editor_inside_the_header_cell() {
    use egui::{Event, Modifiers, PointerButton, accesskit::Role};
    let (mut app, source, sheet) = open(SOURCE);
    let header = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 1, column 1")
        .rect();
    let pos = header.center();
    app.harness.hover_at(pos);
    app.harness.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            app.harness.input_mut().events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    }
    app.harness.run_steps(3);
    assert!(app.harness.state().core.worksheets.views[&sheet].editing);
    let input = app.harness.get_by_label("Row 1, column 1").rect();
    assert!(
        input.top() >= header.top() - 0.5 && input.bottom() <= header.bottom() + 0.5,
        "caption editor {input:?} should fit within header {header:?}"
    );
    app.harness.event(Event::Text(" changed".into()));
    app.harness.run_steps(3);
    assert!(text(&app, source).contains("Name changed"));
    action(&mut app, sheet, json!({"op":"finish"}));
    resize_action(&mut app, sheet, true, 0, 75.0);
    let header = app
        .harness
        .get_by_role_and_label(Role::Button, "Row 1, column 1")
        .rect();
    action(&mut app, sheet, json!({"op":"edit","row":0,"column":0}));
    let input = app.harness.get_by_label("Row 1, column 1").rect();
    assert!(
        input.top() >= header.top() - 0.5 && input.bottom() <= header.bottom() + 0.5,
        "resized header editor {input:?} should fit inside {header:?}"
    );
}
