//! Compact worksheet UI. Mutations are returned through registered commands.
use super::{Actions, Kit, icon_button};
use crate::{
    core::AppCore,
    icons::Icon,
    markdown_table::{
        DEFAULT_ROW_HEIGHT, ResizeAxis, ResizeDrag, RowDrag, SelectionKind, Worksheet,
    },
    t,
    theme::Tok,
};
use egui::{Key, Modifiers};
use notoir_core::{
    command::{CommandInvocation, InvocationSource},
    ids::ViewId,
};
use notoir_editor::Range;
use notoir_editor::markdown_table::Table;
use serde_json::{Value, json};

fn action(actions: &mut Actions, view: ViewId, mut args: Value) {
    args["view"] = json!(view.0);
    actions.push(
        CommandInvocation::new("markdown.tableAction", InvocationSource::Toolbar).with_args(args),
    );
}

pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    view: ViewId,
    actions: &mut Actions,
) {
    core.worksheet_prepare(view);
    let Some(mut state) = core.worksheets.views.get(&view).cloned() else {
        return;
    };
    let Some(v) = core.workbench.view(view) else {
        return;
    };
    let Some(doc) = core.workbench.document(v.document_id) else {
        return;
    };
    let link_open = core.markdown.insert_dialog.as_ref().is_some_and(|d| {
        d.view == view && d.kind == crate::markdown_insert::InsertDialogKind::Link
    });
    let enabled = state.available && !doc.is_read_only();
    let revision = doc.revision;
    if !enabled {
        state.row_drag = None;
    }
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, kit.color(Tok::BgEditor));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(
            ui,
            kit,
            Icon::FileCode,
            &t!("markdown:worksheetSource"),
            false,
            true,
        )
        .clicked()
        {
            action(actions, view, json!({"op":"source"}));
        }
        for (icon, key, value) in [
            (Icon::Bold, "markdown:worksheetBold", "bold"),
            (Icon::Italic, "markdown:worksheetItalic", "italic"),
            (Icon::Strikethrough, "markdown:worksheetStrike", "strike"),
            (Icon::Code, "markdown:worksheetCode", "code"),
        ] {
            if icon_button(ui, kit, icon, &t!(key), false, enabled).clicked() {
                action(
                    actions,
                    view,
                    json!({"op":"format","value":value,"revision":revision}),
                );
            }
        }
        if icon_button(
            ui,
            kit,
            Icon::Link,
            &t!("markdown:worksheetLink"),
            link_open,
            enabled,
        )
        .clicked()
        {
            actions.push(
                CommandInvocation::new("markdown.insertLink", InvocationSource::Toolbar)
                    .with_args(json!({"view":view.0})),
            );
        }
        let structure = icon_button(
            ui,
            kit,
            Icon::Table2,
            &t!("markdown:worksheetStructure"),
            false,
            enabled,
        );
        egui::Popup::menu(&structure).show(|ui| {
            for (op, key) in [
                ("rowBefore", "markdown:worksheetRowBefore"),
                ("rowAfter", "markdown:worksheetRowAfter"),
                ("deleteRows", "markdown:worksheetDeleteRows"),
                ("columnBefore", "markdown:worksheetColumnBefore"),
                ("columnAfter", "markdown:worksheetColumnAfter"),
                ("deleteColumns", "markdown:worksheetDeleteColumns"),
            ] {
                let can_delete = match op {
                    "rowBefore" => state.bounds().0 > 0,
                    "deleteRows" => state.rows().iter().any(|r| *r > 0),
                    "deleteColumns" => state
                        .table
                        .as_ref()
                        .is_some_and(|t| state.columns().len() < t.aligns.len()),
                    _ => true,
                };
                if ui
                    .add_enabled(enabled && can_delete, egui::Button::new(t!(key)))
                    .clicked()
                {
                    action(actions, view, json!({"op":op,"revision":revision}));
                    ui.close();
                }
            }
        });
        let alignment = icon_button(
            ui,
            kit,
            Icon::AlignLeft,
            &t!("markdown:worksheetAlignment"),
            false,
            enabled,
        );
        egui::Popup::menu(&alignment).show(|ui| {
            for (value, key) in [
                ("none", "markdown:worksheetAlignNone"),
                ("left", "markdown:worksheetAlignLeft"),
                ("center", "markdown:worksheetAlignCenter"),
                ("right", "markdown:worksheetAlignRight"),
            ] {
                if ui
                    .add_enabled(enabled, egui::Button::new(t!(key)))
                    .clicked()
                {
                    action(
                        actions,
                        view,
                        json!({"op":"align","value":value,"revision":revision}),
                    );
                    ui.close();
                }
            }
        });
    });
    ui.separator();
    if !state.available {
        ui.colored_label(
            kit.color(Tok::TextSecondary),
            t!("markdown:worksheetUnavailable"),
        );
    } else if !enabled {
        ui.label(t!("markdown:worksheetReadOnly"));
    }
    let Some(table) = state.table.clone() else {
        core.worksheets.views.insert(view, state);
        return;
    };
    let grid_id = ui.id().with(("worksheet-grid", view));
    let grid = ui.interact(
        ui.available_rect_before_wrap(),
        grid_id,
        egui::Sense::focusable_noninteractive(),
    );
    grid.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, t!("markdown:worksheetGrid"))
    });
    if state.focus && !state.editing && !link_open && !egui::Popup::is_any_open(ui.ctx()) {
        ui.memory_mut(|m| m.request_focus(grid_id));
        state.focus = false;
    }
    let focused = ui.memory(|m| m.has_focus(grid_id));
    state.keyboard_focus = focused && !egui::Popup::is_any_open(ui.ctx());
    if focused && !state.editing && !link_open && !egui::Popup::is_any_open(ui.ctx()) {
        let (r, c) = state.selected;
        let shift = ui.input(|i| i.modifiers.shift);
        for (key, dr, dc) in [
            (Key::ArrowUp, -1, 0),
            (Key::ArrowDown, 1, 0),
            (Key::ArrowLeft, 0, -1),
            (Key::ArrowRight, 0, 1),
            (Key::Tab, 0, if shift { -1 } else { 1 }),
        ] {
            if ui.input_mut(|i| {
                i.consume_key(
                    if shift {
                        Modifiers::SHIFT
                    } else {
                        Modifiers::NONE
                    },
                    key,
                )
            }) {
                let mut row = r.saturating_add_signed(dr).min(table.rows.len() - 1);
                let mut col = c.saturating_add_signed(dc).min(table.aligns.len() - 1);
                if key == Key::Tab && !shift && c + 1 == table.aligns.len() {
                    row = (r + 1).min(table.rows.len() - 1);
                    col = 0;
                }
                if key == Key::Tab && shift && c == 0 {
                    row = r.saturating_sub(1);
                    col = table.aligns.len() - 1;
                }
                action(
                    actions,
                    view,
                    json!({"op":"select","row":row,"column":col,"extend":shift && key!=Key::Tab,"reveal":true}),
                );
            }
        }
        for key in [Key::Enter, Key::F2] {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) && enabled {
                action(actions, view, json!({"op":"edit"}));
            }
        }
        for key in [Key::Delete, Key::Backspace] {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) && enabled {
                action(actions, view, json!({"op":"clear","revision":revision}));
            }
        }
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Copy => action(actions, view, json!({"op":"copy"})),
                egui::Event::Cut => action(actions, view, json!({"op":"cut","revision":revision})),
                egui::Event::Paste(value) => action(
                    actions,
                    view,
                    json!({"op":"paste","value":value,"revision":revision}),
                ),
                _ => {}
            }
        }
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let gutter = (table.rows.len().to_string().len() as f32 * 8.0 + 16.0).max(44.0);
    let viewport = ui.available_rect_before_wrap();
    if state.row_drag.is_some() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        state.row_drag = None;
    }
    if state
        .row_drag
        .as_ref()
        .is_some_and(|drag| drag.revision != revision)
    {
        state.row_drag = None;
    }
    if state
        .resize_drag
        .as_ref()
        .is_some_and(|drag| drag.revision != revision)
        || !state.available
    {
        state.resize_drag = None;
    }
    if state.resize_drag.is_some() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        let drag = state.resize_drag.take().expect("checked");
        resize_action(ui, actions, view, &drag, drag.restore);
        state.focus = state.editing;
    }
    let header_height = state
        .row_height(0)
        .min((viewport.height() - DEFAULT_ROW_HEIGHT).max(DEFAULT_ROW_HEIGHT));
    let total_width = gutter + state.layout.column_widths.iter().sum::<f32>();
    let mut row_tops = vec![0.0_f32];
    for r in 1..table.rows.len() {
        row_tops.push(row_tops.last().copied().unwrap_or(0.0) + state.row_height(r));
    }
    let mut grid = Grid {
        kit,
        view,
        revision,
        enabled,
        grid_id,
        gutter,
        viewport,
        header_height,
        actions,
        drop: None,
    };
    let (header_rect, _) = ui.allocate_exact_size(
        egui::vec2(viewport.width(), header_height),
        egui::Sense::hover(),
    );
    let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        egui::pos2(header_rect.left() - state.scroll.0, header_rect.top()),
        egui::vec2(total_width, header_height),
    )));
    header_ui.set_clip_rect(header_rect.intersect(ui.clip_rect()));
    grid.row(&mut header_ui, &mut state, &table, 0);
    let mut offset = egui::vec2(state.scroll.0, state.scroll.1);
    if ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| header_rect.contains(p))
    }) {
        offset.x = (offset.x - ui.input(|i| i.smooth_scroll_delta.x)).max(0.0);
    }
    let body = ui.available_rect_before_wrap();
    if state.reveal {
        let x = state
            .layout
            .column_widths
            .iter()
            .take(state.selected.1)
            .sum::<f32>();
        let width = state.column_width(state.selected.1);
        let y = row_tops[state.selected.0.saturating_sub(1)];
        let height = state.row_height(state.selected.0);
        let visible_width = (body.width() - gutter).max(1.0);
        if x < offset.x || width >= visible_width {
            offset.x = x;
        } else if x + width > offset.x + visible_width {
            offset.x = (x + width - visible_width).max(0.0);
        }
        if state.selected.0 > 0 {
            if y < offset.y || height >= body.height() {
                offset.y = y;
            } else if y + height > offset.y + body.height() {
                offset.y = (y + height - body.height()).max(0.0);
            }
        }
        state.reveal = false;
    }
    if state.row_drag.is_some()
        && ui.input(|i| i.pointer.primary_down())
        && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
        && viewport.contains(pointer)
    {
        let speed = if pointer.y < body.top() + 32.0 {
            -400.0
        } else if pointer.y > body.bottom() - 32.0 {
            400.0
        } else {
            0.0
        };
        offset.y = (offset.y + speed * ui.input(|i| i.stable_dt.min(0.1))).max(0.0);
        if speed != 0.0 {
            ui.ctx().request_repaint();
        }
    }
    let output = egui::ScrollArea::both()
        .id_salt(("worksheet-scroll", view))
        .scroll_offset(offset)
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, visible| {
            let origin = ui.cursor().min;
            let count = table.rows.len().saturating_sub(1);
            let total_height = row_tops.last().copied().unwrap_or(0.0);
            ui.set_min_size(egui::vec2(total_width, total_height));
            let first = row_tops
                .partition_point(|y| *y <= visible.top())
                .saturating_sub(1)
                .min(count);
            let end = row_tops
                .partition_point(|y| *y < visible.bottom())
                .min(count);
            if first < end {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, origin.y + row_tops[first]),
                    egui::vec2(total_width, row_tops[end] - row_tops[first]),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    for index in first..end {
                        grid.row(ui, &mut state, &table, index + 1);
                    }
                });
            }
        });
    if let Some(drag) = state.row_drag.clone() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some((before, y)) = grid.drop {
            let y = y.max(header_rect.bottom()).min(viewport.bottom());
            let right = viewport
                .right()
                .min(viewport.left() + total_width - output.state.offset.x);
            ui.painter().line_segment(
                [egui::pos2(viewport.left(), y), egui::pos2(right, y)],
                egui::Stroke::new(2.0, grid.kit.color(Tok::Accent)),
            );
            if ui.input(|i| i.pointer.primary_released()) {
                action(
                    grid.actions,
                    view,
                    json!({"op":"moveRows","rows":drag.rows,"before":before,"revision":drag.revision}),
                );
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            state.row_drag = None;
        }
    }
    if let Some(drag) = state.resize_drag.clone() {
        ui.ctx()
            .set_cursor_icon(if drag.axis == ResizeAxis::Column {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            });
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let pointer = if drag.axis == ResizeAxis::Column {
                pointer.x
            } else {
                pointer.y
            };
            let size = drag.axis.constrain(drag.original + pointer - drag.pointer);
            let current = if drag.axis == ResizeAxis::Column {
                state.column_width(drag.index)
            } else {
                state.row_height(drag.index)
            };
            if (current - size).abs() > 0.1 {
                resize_action(ui, grid.actions, view, &drag, size);
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            state.resize_drag = None;
            state.focus = state.editing;
        }
        ui.ctx().request_repaint();
    }
    state.scroll = (output.state.offset.x, output.state.offset.y);
    core.worksheets.views.insert(view, state);
}

fn resize_action(ui: &egui::Ui, actions: &mut Actions, view: ViewId, drag: &ResizeDrag, size: f32) {
    action(
        actions,
        view,
        json!({"op":if drag.axis==ResizeAxis::Row {"resizeRow"}else{"resizeColumn"},"row":drag.index,"column":drag.index,"size":size,"revision":drag.revision}),
    );
    ui.ctx().request_repaint();
}

struct Grid<'a, 'theme> {
    kit: &'a mut Kit<'theme>,
    view: ViewId,
    revision: u64,
    enabled: bool,
    grid_id: egui::Id,
    gutter: f32,
    viewport: egui::Rect,
    header_height: f32,
    actions: &'a mut Actions,
    drop: Option<(usize, f32)>,
}
impl Grid<'_, '_> {
    fn auto_size(
        &mut self,
        ui: &egui::Ui,
        state: &Worksheet,
        table: &Table,
        axis: ResizeAxis,
        index: usize,
    ) -> f32 {
        let mut size = 0.0_f32;
        let cells: Vec<_> = if axis == ResizeAxis::Row {
            (0..table.aligns.len()).map(|c| (index, c)).collect()
        } else {
            (0..table.rows.len()).map(|r| (r, index)).collect()
        };
        for (r, c) in cells {
            let mut job =
                super::md_preview::worksheet_job(&table.rows[r][c].markdown, r == 0, self.kit);
            job.wrap.max_width = if axis == ResizeAxis::Row {
                state.column_width(c) - 16.0
            } else {
                f32::INFINITY
            };
            job.wrap.max_rows = if axis == ResizeAxis::Row {
                usize::MAX
            } else {
                1
            };
            job.wrap.break_anywhere = false;
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            size = size.max(if axis == ResizeAxis::Row {
                galley.size().y + 10.0
            } else {
                galley.size().x + 16.0
            });
        }
        axis.constrain(size)
    }
    fn resize_handle(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut Worksheet,
        table: &Table,
        axis: ResizeAxis,
        index: usize,
        rect: egui::Rect,
    ) {
        if !rect.intersects(ui.clip_rect()) {
            return;
        }
        let size = if axis == ResizeAxis::Column {
            state.column_width(index)
        } else {
            state.row_height(index)
        };
        let label = if axis == ResizeAxis::Column {
            t!("markdown:worksheetResizeColumn", column = index + 1)
        } else {
            t!("markdown:worksheetResizeRow", row = index + 1)
        };
        let response = ui.interact(
            rect.intersect(ui.clip_rect()),
            ui.id()
                .with(("worksheet-size", axis == ResizeAxis::Column, index)),
            if state.available {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            },
        );
        response.widget_info(|| egui::WidgetInfo::slider(state.available, size as f64, &label));
        state.keyboard_focus |= response.has_focus() && !egui::Popup::is_any_open(ui.ctx());
        let response = response.on_hover_text(t!("markdown:worksheetResizeHint"));
        if response.clicked() {
            response.request_focus();
        }
        if response.hovered() && state.available {
            ui.ctx().set_cursor_icon(if axis == ResizeAxis::Column {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            });
        }
        if response.hovered()
            || response.has_focus()
            || state
                .resize_drag
                .as_ref()
                .is_some_and(|drag| drag.axis == axis && drag.index == index)
        {
            let points = if axis == ResizeAxis::Column {
                [rect.right_top(), rect.right_bottom()]
            } else {
                [rect.left_bottom(), rect.right_bottom()]
            };
            ui.painter()
                .line_segment(points, egui::Stroke::new(1.5, self.kit.color(Tok::Accent)));
        }
        let displayed = if axis == ResizeAxis::Row && index == 0 {
            self.header_height
        } else {
            size
        };
        if response.double_clicked() && state.available {
            let fitted = self.auto_size(ui, state, table, axis, index);
            let drag = ResizeDrag {
                axis,
                index,
                original: displayed,
                restore: size,
                pointer: 0.0,
                revision: self.revision,
            };
            resize_action(ui, self.actions, self.view, &drag, fitted);
        } else if response.drag_started()
            && state.available
            && let Some(pointer) = ui.input(|i| i.pointer.press_origin())
        {
            state.row_drag = None;
            state.resize_drag = Some(ResizeDrag {
                axis,
                index,
                original: displayed,
                restore: size,
                pointer: if axis == ResizeAxis::Column {
                    pointer.x
                } else {
                    pointer.y
                },
                revision: self.revision,
            });
            response.request_focus();
        }
        if response.has_focus() && state.available {
            let (less, more) = if axis == ResizeAxis::Column {
                (Key::ArrowLeft, Key::ArrowRight)
            } else {
                (Key::ArrowUp, Key::ArrowDown)
            };
            for (key, delta) in [(less, -8.0), (more, 8.0)] {
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                    let drag = ResizeDrag {
                        axis,
                        index,
                        original: displayed,
                        restore: size,
                        pointer: 0.0,
                        revision: self.revision,
                    };
                    resize_action(
                        ui,
                        self.actions,
                        self.view,
                        &drag,
                        axis.constrain(displayed + delta),
                    );
                }
            }
        }
        if response.secondary_clicked() {
            let selected = if axis == ResizeAxis::Row {
                state.selection_kind == SelectionKind::Rows && state.selected_rows.contains(&index)
            } else {
                state.selection_kind == SelectionKind::Columns
                    && state.selected_columns.contains(&index)
            };
            if !selected {
                action(
                    self.actions,
                    self.view,
                    json!({"op":if axis==ResizeAxis::Row {"selectRows"}else{"selectColumns"},"row":index,"column":index}),
                );
            }
        }
        response.context_menu(|ui| self.structure_menu(ui, state, axis == ResizeAxis::Row));
    }
    fn row(&mut self, ui: &mut egui::Ui, state: &mut Worksheet, table: &Table, r: usize) {
        let height = if r == 0 {
            self.header_height
        } else {
            state.row_height(r)
        };
        ui.push_id(("worksheet-row", self.view, r), |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(height);
                ui.spacing_mut().item_spacing.x = 0.0;
                let clip = ui.clip_rect();
                let top = ui.cursor().top();
                let rect = egui::Rect::from_min_size(
                    egui::pos2(clip.left(), top),
                    egui::vec2(self.gutter, height),
                );
                ui.allocate_exact_size(egui::vec2(self.gutter, height), egui::Sense::hover());
                // Clip cells behind the fixed row-number gutter during horizontal scrolling.
                ui.set_clip_rect(clip.intersect(egui::Rect::from_min_max(
                    egui::pos2(rect.right(), clip.top()),
                    clip.max,
                )));
                ui.add_enabled_ui(state.available, |ui| {
                    let mut edge = ui.cursor().left();
                    for c in 0..table.aligns.len() {
                        ui.push_id(c, |ui| self.cell(ui, state, table, r, c, height));
                    }
                    if r == 0 {
                        for c in 0..table.aligns.len() {
                            edge += state.column_width(c);
                            self.resize_handle(
                                ui,
                                state,
                                table,
                                ResizeAxis::Column,
                                c,
                                egui::Rect::from_min_size(
                                    egui::pos2(edge - 5.0, top),
                                    egui::vec2(5.0, height),
                                ),
                            );
                        }
                    }
                });
                ui.set_clip_rect(clip);
                self.counter(ui, state, r, rect);
                self.resize_handle(
                    ui,
                    state,
                    table,
                    ResizeAxis::Row,
                    r,
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left(), rect.bottom() - 5.0),
                        egui::vec2(rect.width(), 5.0),
                    ),
                );
                if state.row_drag.is_some()
                    && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
                    && self.viewport.contains(pointer)
                {
                    let row_zone = egui::Rect::from_min_max(
                        egui::pos2(self.viewport.left(), top - 2.0),
                        egui::pos2(self.viewport.right(), top + height + 2.0),
                    )
                    .intersect(clip);
                    if row_zone.contains(pointer) {
                        self.drop = Some(if r == 0 {
                            (1, rect.bottom())
                        } else if pointer.y < rect.center().y {
                            (r, rect.top())
                        } else {
                            (r + 1, rect.bottom())
                        });
                    } else if r + 1 == table.rows.len()
                        && pointer.y > rect.bottom()
                        && clip.contains(pointer)
                    {
                        self.drop = Some((table.rows.len(), rect.bottom()));
                    }
                }
            });
        });
    }
    fn select_axis(&mut self, ui: &egui::Ui, row: bool, index: usize, preserve: bool) {
        if preserve {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        action(
            self.actions,
            self.view,
            json!({"op":if row {"selectRows"}else{"selectColumns"},"row":index,"column":index,"extend":mods.shift,"toggle":mods.command||mods.ctrl}),
        );
    }
    fn begin_drag(&mut self, state: &mut Worksheet, row: usize) {
        if !self.enabled || row == 0 || state.resize_drag.is_some() || state.row_drag.is_some() {
            return;
        }
        let rows =
            if state.selection_kind == SelectionKind::Rows && state.selected_rows.contains(&row) {
                state.rows()
            } else {
                action(
                    self.actions,
                    self.view,
                    json!({"op":"selectRows","row":row}),
                );
                vec![row]
            };
        if rows.contains(&0) {
            return;
        }
        state.row_drag = Some(RowDrag {
            rows,
            revision: self.revision,
        });
    }
    fn counter(&mut self, ui: &mut egui::Ui, state: &mut Worksheet, row: usize, rect: egui::Rect) {
        let selected =
            state.selection_kind == SelectionKind::Rows && state.selected_rows.contains(&row);
        let response = ui.interact(
            rect.intersect(ui.clip_rect()),
            ui.id().with("row-counter"),
            if state.available {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            },
        );
        let fill = if selected {
            self.kit.color(Tok::AccentSoft)
        } else if response.hovered() {
            self.kit.color(Tok::BgPanelRaised)
        } else {
            self.kit.color(Tok::BgPanel)
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, self.kit.color(Tok::Border)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.intersect(ui.clip_rect()).center(),
            egui::Align2::CENTER_CENTER,
            (row + 1).to_string(),
            crate::fonts::mono(12.0),
            self.kit.color(if selected {
                Tok::Accent
            } else {
                Tok::TextSecondary
            }),
        );
        let label = t!("markdown:worksheetRowSelector", row = row + 1);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, state.available, &label)
        });
        let response = response.on_hover_text(if row == 0 {
            t!("markdown:worksheetFixedHeader")
        } else {
            label
        });
        if response.clicked() {
            self.select_axis(ui, true, row, false);
        }
        if response.secondary_clicked() && !selected {
            action(
                self.actions,
                self.view,
                json!({"op":"selectRows","row":row}),
            );
        }
        if response.drag_started() {
            self.begin_drag(state, row);
        }
        if response.hovered() && selected && self.enabled && !state.selected_rows.contains(&0) {
            ui.ctx().set_cursor_icon(if state.row_drag.is_some() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }
        response.context_menu(|ui| self.structure_menu(ui, state, true));
    }
    fn structure_menu(&mut self, ui: &mut egui::Ui, state: &Worksheet, rows: bool) {
        let items = if rows {
            [
                ("rowBefore", "markdown:worksheetRowBefore"),
                ("rowAfter", "markdown:worksheetRowAfter"),
                ("deleteRows", "markdown:worksheetDeleteRows"),
            ]
        } else {
            [
                ("columnBefore", "markdown:worksheetColumnBefore"),
                ("columnAfter", "markdown:worksheetColumnAfter"),
                ("deleteColumns", "markdown:worksheetDeleteColumns"),
            ]
        };
        for (op, key) in items {
            let allowed = match op {
                "rowBefore" => state.bounds().0 > 0,
                "deleteRows" => state.rows().iter().any(|r| *r > 0),
                "deleteColumns" => state
                    .table
                    .as_ref()
                    .is_some_and(|t| state.columns().len() < t.aligns.len()),
                _ => true,
            };
            if ui
                .add_enabled(self.enabled && allowed, egui::Button::new(t!(key)))
                .clicked()
            {
                action(
                    self.actions,
                    self.view,
                    json!({"op":op,"revision":self.revision}),
                );
                ui.close();
            }
        }
    }
    fn cell(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut Worksheet,
        table: &Table,
        r: usize,
        c: usize,
        height: f32,
    ) {
        let width = state.column_width(c);
        let raw = &table.rows[r][c].markdown;
        let label = t!("markdown:worksheetCell", row = r + 1, column = c + 1);
        if state.editing && state.selected == (r, c) && self.enabled {
            let mut value = raw.clone();
            ui.input_mut(|input| {
                for event in &mut input.events {
                    if let egui::Event::Paste(text) = event {
                        *text = text.replace(['\r', '\n', '\t'], " ");
                    }
                }
            });
            let tab = ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Tab)
                    || i.consume_key(Modifiers::SHIFT, Key::Tab)
            });
            let finish = ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Escape)
                    || i.consume_key(Modifiers::NONE, Key::Enter)
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            let output = egui::ScrollArea::vertical()
                .id_salt(("cell-edit-scroll", self.view, r, c))
                .min_scrolled_height(0.0)
                .max_height(height)
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    egui::TextEdit::multiline(&mut value)
                        .id(ui.id().with("edit"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(1)
                        .min_size(egui::vec2(0.0, height))
                        .show(ui)
                })
                .inner;
            output.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
            });
            if state.focus {
                output.response.request_focus();
                state.focus = false;
            }
            state.keyboard_focus |= output.response.has_focus();
            if let Some(range) = output.cursor_range {
                state.text_selection =
                    Some(Range::new(range.secondary.index.0, range.primary.index.0));
            }
            if output.response.changed() {
                action(
                    self.actions,
                    self.view,
                    json!({"op":"set","value":value,"revision":self.revision}),
                );
            }
            if tab || finish {
                action(self.actions, self.view, json!({"op":"finish"}));
                if tab {
                    let back = ui.input(|i| i.modifiers.shift);
                    let (row, col) = if back {
                        if c == 0 {
                            (r.saturating_sub(1), table.aligns.len() - 1)
                        } else {
                            (r, c - 1)
                        }
                    } else if c + 1 == table.aligns.len() {
                        ((r + 1).min(table.rows.len() - 1), 0)
                    } else {
                        (r, c + 1)
                    };
                    action(
                        self.actions,
                        self.view,
                        json!({"op":"select","row":row,"column":col,"reveal":true}),
                    );
                }
                ui.memory_mut(|m| m.request_focus(self.grid_id));
            }
            return;
        }
        let selected = state.contains(r, c);
        let mut job = super::md_preview::worksheet_job(raw, r == 0, self.kit);
        job.wrap.max_width = width - 16.0;
        job.wrap.max_rows = usize::MAX;
        job.wrap.break_anywhere = false;
        job.halign = match table.aligns[c] {
            notoir_editor::markdown::Align::Center => egui::Align::Center,
            notoir_editor::markdown::Align::Right => egui::Align::Max,
            _ => egui::Align::Min,
        };
        if !state.available {
            for section in &mut job.sections {
                section.format.color = self.kit.color(Tok::TextDisabled);
            }
        }
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else if response.hovered() {
            self.kit.color(Tok::BgPanelRaised)
        } else if r == 0 {
            self.kit.color(Tok::BgPanel)
        } else {
            self.kit.color(Tok::BgEditor)
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                1.0,
                self.kit
                    .color(if selected { Tok::Accent } else { Tok::Border }),
            ),
            egui::StrokeKind::Inside,
        );
        let x = match table.aligns[c] {
            notoir_editor::markdown::Align::Center => rect.center().x,
            notoir_editor::markdown::Align::Right => rect.right() - 8.0,
            _ => rect.left() + 8.0,
        };
        ui.painter()
            .with_clip_rect(rect.shrink2(egui::vec2(6.0, 4.0)).intersect(ui.clip_rect()))
            .galley(
                egui::pos2(x, rect.top() + 5.0),
                galley,
                self.kit.color(Tok::TextPrimary),
            );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, state.available, &label)
        });
        let response = response.on_hover_text(if r == 0 {
            t!("markdown:worksheetColumnSelector", column = c + 1)
        } else {
            raw.clone()
        });
        if response.clicked() {
            if response.double_clicked() {
                action(
                    self.actions,
                    self.view,
                    json!({"op":"edit","row":r,"column":c}),
                );
            } else if r == 0 {
                self.select_axis(ui, false, c, false);
            } else {
                action(
                    self.actions,
                    self.view,
                    json!({"op":"select","row":r,"column":c,"extend":ui.input(|i|i.modifiers.shift)}),
                );
            }
            ui.memory_mut(|m| m.request_focus(self.grid_id));
        }
        if r == 0
            && response.secondary_clicked()
            && !(state.selection_kind == SelectionKind::Columns
                && state.selected_columns.contains(&c))
        {
            action(
                self.actions,
                self.view,
                json!({"op":"selectColumns","column":c}),
            );
        }
        if r > 0 && state.selection_kind == SelectionKind::Rows && state.selected_rows.contains(&r)
        {
            if response.drag_started() {
                self.begin_drag(state, r);
            }
            if response.hovered() && self.enabled && !state.selected_rows.contains(&0) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
        response.context_menu(|ui| {
            if r == 0 {
                self.structure_menu(ui, state, false);
            } else {
                for (op, key) in [
                    ("copy", "markdown:worksheetCopy"),
                    ("cut", "markdown:worksheetCut"),
                    ("clear", "markdown:worksheetClear"),
                ] {
                    if ui
                        .add_enabled(op == "copy" || self.enabled, egui::Button::new(t!(key)))
                        .clicked()
                    {
                        action(
                            self.actions,
                            self.view,
                            json!({"op":op,"revision":self.revision}),
                        );
                        ui.close();
                    }
                }
            }
        });
    }
}
