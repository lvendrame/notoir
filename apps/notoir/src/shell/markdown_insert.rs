//! The link/image dialog and searchable fenced-code language picker.
use super::{Actions, Kit};
use crate::markdown_insert::InsertDialogKind;
use crate::{core::AppCore, t};
use notoir_core::command::{CommandInvocation, InvocationSource};
use serde_json::json;

pub fn show(ctx: &egui::Context, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    let Some(mut dialog) = core.markdown.insert_dialog.take() else {
        return;
    };
    let valid = core.md_insert_target_valid(&dialog);
    let mut confirm = false;
    let mut language = None;
    let mut choose_file = None;
    let mut cancel = false;
    let modal = egui::Modal::new(egui::Id::new("markdown-insert-dialog")).show(ctx, |ui| {
        ui.set_width(420.0_f32.min(ui.available_width()));
        ui.heading(t!(if dialog.kind == InsertDialogKind::Code {
            "markdown:insert.code"
        } else if dialog.kind == InsertDialogKind::Image {
            "markdown:insert.image"
        } else if dialog.kind == InsertDialogKind::Table {
            "markdown:insert.table"
        } else {
            "markdown:insert.link"
        }));
        ui.add_space(8.0);
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let navigating =
            (dialog.kind == InsertDialogKind::Code) || dialog.destination.starts_with('@');
        let down = navigating
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = navigating
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        if dialog.kind == InsertDialogKind::Code {
            let response = ui.add(
                egui::TextEdit::singleline(&mut dialog.destination)
                    .desired_width(f32::INFINITY)
                    .hint_text(t!("markdown:insert.searchLanguages")),
            );
            if dialog.fresh {
                response.request_focus();
            }
            if response.changed() {
                dialog.selected = 0;
            }
            let query = dialog.destination.trim();
            let mut rows = vec![(String::new(), t!("markdown:insert.general"))];
            rows.extend(
                notoir_editor::language::all()
                    .into_iter()
                    .filter(|l| l.id.as_str() != "plaintext")
                    .map(|l| (l.id.as_str().to_owned(), l.name.to_owned())),
            );
            rows.retain(|(_, name)| {
                query.is_empty() || notoir_core::fuzzy::fuzzy_match(query, name).is_some()
            });
            rows.sort_by(|a, b| {
                (!a.0.is_empty())
                    .cmp(&(!b.0.is_empty()))
                    .then(a.1.cmp(&b.1))
            });
            navigate(&mut dialog.selected, rows.len(), down, up);
            if rows.is_empty() {
                ui.label(t!("markdown:insert.noResults"));
            }
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .min_scrolled_height(240.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, (id, name)) in rows.iter().enumerate() {
                        let row = ui.add_enabled(
                            valid,
                            egui::Button::selectable(i == dialog.selected, name),
                        );
                        if i == dialog.selected
                            && (down || up || dialog.fresh || response.changed())
                        {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                        if row.clicked() && valid {
                            language = Some(id.clone());
                            confirm = true;
                        }
                    }
                });
            if enter
                && valid
                && let Some((id, _)) = rows.get(dialog.selected)
            {
                language = Some(id.clone());
                confirm = true;
            }
        } else if dialog.kind == InsertDialogKind::Table {
            let columns_label = ui.label(t!("markdown:insert.columns"));
            let columns = ui
                .add(egui::DragValue::new(&mut dialog.columns).range(1..=100))
                .labelled_by(columns_label.id);
            if dialog.fresh {
                columns.request_focus();
            }
            let rows_label = ui.label(t!("markdown:insert.rows"));
            ui.add(egui::DragValue::new(&mut dialog.rows).range(1..=100))
                .labelled_by(rows_label.id);
            ui.label(t!("markdown:insert.tableHint"));
            let can_insert =
                valid && (1..=100).contains(&dialog.columns) && (1..=100).contains(&dialog.rows);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_insert, egui::Button::new(t!("markdown:insert.menu")))
                    .clicked()
                {
                    confirm = true;
                }
                if ui.button(t!("common:cancel")).clicked() {
                    cancel = true;
                }
            });
            if can_insert && enter {
                confirm = true;
            }
        } else {
            let caption = t!(if dialog.kind == InsertDialogKind::Image {
                "markdown:insert.altText"
            } else {
                "markdown:insert.caption"
            });
            let label = ui.label(&caption);
            let response = ui.add(
                egui::TextEdit::singleline(&mut dialog.caption)
                    .desired_width(f32::INFINITY)
                    .hint_text(&caption),
            );
            let response = response.labelled_by(label.id);
            if dialog.fresh {
                response.request_focus();
            }
            let label = ui.label(t!("markdown:insert.destination"));
            let destination = ui.add(
                egui::TextEdit::singleline(&mut dialog.destination)
                    .desired_width(f32::INFINITY)
                    .hint_text(t!("markdown:insert.destinationHint")),
            );
            let destination = destination.labelled_by(label.id);
            if destination.changed() {
                dialog.selected = 0;
            }
            let completion = dialog.destination.starts_with('@');
            if completion {
                let rows = core.md_file_candidates(
                    &dialog.destination[1..],
                    dialog.kind == InsertDialogKind::Image,
                );
                if core.workspace.is_none() {
                    ui.label(t!("markdown:insert.noWorkspace"));
                } else if core.workspace.as_ref().is_some_and(|w| !w.index_ready) {
                    ui.label(t!("markdown:insert.indexing"));
                } else if rows.is_empty() {
                    ui.label(t!("markdown:insert.noResults"));
                }
                navigate(&mut dialog.selected, rows.len(), down, up);
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .min_scrolled_height(120.0)
                    .show(ui, |ui| {
                        for (i, path) in rows.iter().enumerate() {
                            let row = ui.selectable_label(i == dialog.selected, path);
                            if i == dialog.selected
                                && (down || up || dialog.fresh || destination.changed())
                            {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() {
                                choose_file = Some(path.clone());
                                destination.request_focus();
                            }
                        }
                    });
                if enter && let Some(path) = rows.get(dialog.selected) {
                    choose_file = Some(path.clone());
                    destination.request_focus();
                }
            }
            let can_insert = valid && !completion && !dialog.destination.trim().is_empty();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_insert, egui::Button::new(t!("markdown:insert.menu")))
                    .clicked()
                {
                    confirm = true;
                }
                if ui.button(t!("common:cancel")).clicked() {
                    cancel = true;
                }
            });
            if !completion && can_insert && enter {
                confirm = true;
            }
        }
        if dialog.kind == InsertDialogKind::Code && ui.button(t!("common:cancel")).clicked() {
            cancel = true;
        }
        if !valid || dialog.error {
            ui.colored_label(
                kit.color(crate::theme::Tok::Danger),
                t!("markdown:insert.staleTarget"),
            );
        }
    });
    dialog.fresh = false;
    if cancel || modal.should_close() {
        core.focus_md_insert_target(&dialog);
    } else {
        core.markdown.insert_dialog = Some(dialog);
        if let Some(file) = choose_file {
            core.choose_md_file(&file);
        }
        if confirm {
            actions.push(
                CommandInvocation::new("markdown.confirmInsert", InvocationSource::Menu)
                    .with_args(json!({"language": language})),
            );
        }
    }
}

fn navigate(selected: &mut usize, count: usize, down: bool, up: bool) {
    *selected = (*selected).min(count.saturating_sub(1));
    if down {
        *selected = (*selected + 1).min(count.saturating_sub(1));
    }
    if up {
        *selected = selected.saturating_sub(1);
    }
}
