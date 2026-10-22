//! Virtualized CSV table; all document mutations use registered commands.
use super::{Actions, Kit};
use crate::{core::AppCore, t};
use egui::{RichText, Vec2};
use notoir_core::{
    command::{CommandInvocation, InvocationSource},
    ids::ViewId,
};
use serde_json::{Value, json};
fn action(actions: &mut Actions, view: ViewId, mut args: Value) {
    args["view"] = json!(view.0);
    actions.push(CommandInvocation::new("csv.action", InvocationSource::Toolbar).with_args(args));
}
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    view: ViewId,
    actions: &mut Actions,
) {
    core.csv_prepare(view);
    let Some(state) = core.csv.views.get_mut(&view) else {
        return;
    };
    if state.too_large {
        ui.label(t!("csv:tooLarge"));
        return;
    }
    ui.push_id(view.0, |ui| {
        ui.horizontal_wrapped(|ui| {
            let mut header = state.header;
            let header_response = ui.checkbox(&mut header, t!("csv:header"));
            crate::live::track(&header_response, "csv.header");
            if header_response.changed() {
                action(actions, view, json!({"op":"header","value":header}));
            }
            let mut separator = state.separator;
            let separator_response = egui::ComboBox::from_id_salt("separator")
                .selected_text(match separator {
                    None => t!("csv:auto"),
                    Some(';') => t!("csv:semicolon"),
                    Some('\t') => t!("csv:tab"),
                    _ => t!("csv:comma"),
                })
                .show_ui(ui, |ui| {
                    for (value, key) in [
                        (None, "csv:auto"),
                        (Some(','), "csv:comma"),
                        (Some(';'), "csv:semicolon"),
                        (Some('\t'), "csv:tab"),
                    ] {
                        let r = ui.selectable_value(&mut separator, value, t!(key));
                        crate::live::track(&r, key);
                    }
                });
            crate::live::track(&separator_response.response, "csv.separator");
            if separator != state.separator {
                action(
                    actions,
                    view,
                    json!({"op":"separator","value":separator.map(|c| c.to_string())}),
                );
            }
            if let Some(table) = &state.parsed {
                let columns_response = ui.menu_button(t!("csv:columns"), |ui| {
                    let r = ui.button(t!("csv:showAll"));
                    crate::live::track(&r, "csv.showAll");
                    if r.clicked() {
                        action(actions, view, json!({"op":"showAll"}));
                    }
                    for c in 0..table.columns {
                        let label = heading(state.header, table, c);
                        let mut visible = !state.hidden.contains(&c);
                        let r = ui.checkbox(&mut visible, label);
                        crate::live::track(&r, &format!("csv.column.{c}"));
                        if r.changed() {
                            action(
                                actions,
                                view,
                                json!({"op":"visible","column":c,"value":visible}),
                            );
                        }
                    }
                });
                crate::live::track(&columns_response.response, "csv.columns");
            }
        });
        ui.horizontal(|ui| {
            let mut search = state.search.clone();
            let label = t!("csv:search");
            let r = ui.add(
                egui::TextEdit::singleline(&mut search)
                    .hint_text(&label)
                    .desired_width(170.0),
            );
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
            crate::live::track(&r, "csv.search");
            if r.changed() {
                action(actions, view, json!({"op":"search","value":search}));
            }
            let r = ui.button(t!("csv:previous"));
            crate::live::track(&r, "csv.previous");
            if r.clicked() {
                action(actions, view, json!({"op":"previous"}));
            }
            let r = ui.button(t!("csv:next"));
            crate::live::track(&r, "csv.next");
            if r.clicked() {
                action(actions, view, json!({"op":"next"}));
            }
        });
        if let Some(e) = &state.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                t!("csv:invalid", line = e.line, column = e.column),
            );
        }
        if state.pending.is_some() {
            ui.label(t!("csv:loading"));
        }
        let Some(table) = state.parsed.clone() else {
            return;
        };
        let rows = state.rows.clone();
        let matches = &state.found;
        if !state.search.is_empty() {
            ui.label(t!("csv:matches", count = matches.len()));
        }
        let selected = matches
            .get(state.selected_match % matches.len().max(1))
            .copied();
        let stale = state.pending.is_some() || state.error.is_some();
        let columns: Vec<_> = (0..table.columns)
            .filter(|c| !state.hidden.contains(c))
            .collect();
        if columns.is_empty() {
            ui.label(t!("csv:noColumns"));
            return;
        }
        let width = 170.0;
        let mut horizontal = egui::ScrollArea::horizontal().id_salt("csv-horizontal");
        if state.scroll_match
            && let Some((_, column)) = selected
            && let Some(index) = columns.iter().position(|&c| c == column)
        {
            horizontal = horizontal.horizontal_scroll_offset(index as f32 * width);
        }
        horizontal.show(ui, |ui| {
            ui.set_min_width(columns.len() as f32 * width);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for &c in &columns {
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 28.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(width);
                            ui.add_sized(
                                [130.0, 24.0],
                                egui::Label::new(
                                    RichText::new(heading(state.header, &table, c)).strong(),
                                )
                                .truncate(),
                            );
                            let label = t!("csv:filter", column = heading(state.header, &table, c));
                            let icon = kit.icon(
                                ui.ctx(),
                                crate::icons::Icon::ChevronDown,
                                14.0,
                                if state.filters.contains_key(&c) {
                                    ui.visuals().selection.bg_fill
                                } else {
                                    ui.visuals().text_color()
                                },
                            );
                            let response = ui.add(egui::Button::image(icon)).on_hover_text(&label);
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label)
                            });
                            crate::live::track(&response, &format!("csv.filter.{c}"));
                            egui::Popup::menu(&response)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    let mut filter =
                                        state.filters.get(&c).cloned().unwrap_or_default();
                                    let r = ui.add(
                                        egui::TextEdit::singleline(&mut filter).hint_text(&label),
                                    );
                                    r.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::TextEdit,
                                            true,
                                            &label,
                                        )
                                    });
                                    crate::live::track(&r, "csv.filterInput");
                                    if r.changed() {
                                        action(
                                            actions,
                                            view,
                                            json!({"op":"filter","column":c,"value":filter}),
                                        );
                                    }
                                    let r = ui.button(t!("csv:clear"));
                                    crate::live::track(&r, "csv.clearFilter");
                                    if r.clicked() {
                                        action(
                                            actions,
                                            view,
                                            json!({"op":"filter","column":c,"value":""}),
                                        );
                                    }
                                    ui.separator();
                                    let hide = ui.button(t!("csv:hideColumn"));
                                    crate::live::track(&hide, "csv.hideColumn");
                                    if hide.clicked() {
                                        action(
                                            actions,
                                            view,
                                            json!({"op":"visible","column":c,"value":false}),
                                        );
                                        ui.close();
                                    }
                                });
                        },
                    );
                }
            });
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("csv-rows")
                .auto_shrink([false, false]);
            if state.scroll_match {
                if let Some((r, _)) = selected
                    && let Some(index) = rows.iter().position(|&x| x == r)
                {
                    scroll = scroll.vertical_scroll_offset(index as f32 * 30.0);
                }
                state.scroll_match = false;
            }
            scroll.show_rows(ui, 26.0, rows.len(), |ui, range| {
                for index in range {
                    let row = rows[index];
                    ui.push_id(row, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for &column in &columns {
                                ui.push_id(column, |ui| {
                                    let value = table.rows[row]
                                        .get(column)
                                        .map_or("", |c| c.value.as_str());
                                    if let Some(editing) = state
                                        .editing
                                        .as_mut()
                                        .filter(|e| e.row == row && e.column == column)
                                    {
                                        let label =
                                            t!("csv:editCell", row = row + 1, column = column + 1);
                                        let response = ui.add_sized(
                                            [width, 26.0],
                                            egui::TextEdit::multiline(&mut editing.value)
                                                .desired_rows(1)
                                                .return_key(Some(egui::KeyboardShortcut::new(
                                                    egui::Modifiers::SHIFT,
                                                    egui::Key::Enter,
                                                ))),
                                        );
                                        response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::TextEdit,
                                                true,
                                                &label,
                                            )
                                        });
                                        crate::live::track(&response, "csv.editCell");
                                        let initial = editing.focus;
                                        if initial {
                                            response.request_focus();
                                            editing.focus = false;
                                        }
                                        if ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                                        }) {
                                            action(actions, view, json!({"op":"cancelEdit"}));
                                        } else if ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                        }) || (response.lost_focus() && !initial)
                                        {
                                            action(
                                                actions,
                                                view,
                                                json!({"op":"commit","value":editing.value}),
                                            );
                                        }
                                    } else {
                                        let matched = !state.search.is_empty()
                                            && value
                                                .to_lowercase()
                                                .contains(&state.search.to_lowercase());
                                        let mut text = RichText::new(value.replace('\n', " ↵ "));
                                        if stale {
                                            text = text.weak();
                                        }
                                        if matched {
                                            text = text
                                                .background_color(ui.visuals().selection.bg_fill);
                                        }
                                        let response = ui
                                            .add_sized(
                                                [width, 26.0],
                                                egui::Button::selectable(
                                                    selected == Some((row, column)),
                                                    text,
                                                )
                                                .frame(true)
                                                .truncate(),
                                            )
                                            .on_hover_text(value);
                                        crate::live::track(
                                            &response,
                                            &format!("csv.cell.{row}.{column}"),
                                        );
                                        if response.clicked() && crate::live::enabled() {
                                            tracing::info!(
                                                row,
                                                column,
                                                double = response.double_clicked(),
                                                stale,
                                                "live CSV cell click"
                                            );
                                        }
                                        if response.double_clicked() && !stale {
                                            action(
                                                actions,
                                                view,
                                                json!({"op":"edit","row":row,"column":column}),
                                            );
                                        }
                                    }
                                });
                            }
                        });
                    });
                }
            });
        });
    });
}
fn heading(header: bool, table: &notoir_editor::csv::Table, c: usize) -> String {
    if header
        && let Some(cell) = table.rows.first().and_then(|r| r.get(c))
        && !cell.value.is_empty()
    {
        return cell.value.clone();
    }
    t!("csv:column", number = c + 1)
}
