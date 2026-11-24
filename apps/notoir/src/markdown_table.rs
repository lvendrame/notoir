//! Worksheet views over the authoritative Markdown document.
use crate::core::{AppCore, Effect};
use notoir_core::{
    command::CommandInvocation,
    ids::ViewId,
    view::{TableBinding, ViewKind},
    workbench::TabPosition,
};
use notoir_editor::{
    ChangeSet, EditKind, Range, Selection,
    markdown::Align,
    markdown_table::{self, Table},
};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionKind {
    #[default]
    Cells,
    Rows,
    Columns,
}

#[derive(Clone, Debug)]
pub struct RowDrag {
    pub rows: Vec<usize>,
    pub revision: u64,
}

pub const DEFAULT_COLUMN_WIDTH: f32 = 184.0;
pub const MIN_COLUMN_WIDTH: f32 = 64.0;
pub const DEFAULT_ROW_HEIGHT: f32 = 30.0;
pub const MAX_COLUMN_WIDTH: f32 = 4096.0;
pub const MAX_ROW_HEIGHT: f32 = 16384.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeAxis {
    Row,
    Column,
}
impl ResizeAxis {
    pub fn constrain(self, size: f32) -> f32 {
        size.clamp(
            if self == Self::Row {
                DEFAULT_ROW_HEIGHT
            } else {
                MIN_COLUMN_WIDTH
            },
            if self == Self::Row {
                MAX_ROW_HEIGHT
            } else {
                MAX_COLUMN_WIDTH
            },
        )
    }
}
#[derive(Clone, Debug)]
pub struct ResizeDrag {
    pub axis: ResizeAxis,
    pub index: usize,
    pub original: f32,
    pub restore: f32,
    pub pointer: f32,
    pub revision: u64,
}
#[derive(Clone, Default)]
pub struct WorksheetLayout {
    pub column_widths: Arc<Vec<f32>>,
    pub row_heights: Arc<Vec<f32>>,
}
impl WorksheetLayout {
    fn resize(&mut self, rows: usize, columns: usize) {
        if self.row_heights.len() != rows {
            Arc::make_mut(&mut self.row_heights).resize(rows, DEFAULT_ROW_HEIGHT);
        }
        if self.column_widths.len() != columns {
            Arc::make_mut(&mut self.column_widths).resize(columns, DEFAULT_COLUMN_WIDTH);
        }
    }
}

#[derive(Clone)]
struct SelectionSnapshot {
    kind: SelectionKind,
    anchor: (usize, usize),
    selected: (usize, usize),
    rows: BTreeSet<usize>,
    columns: BTreeSet<usize>,
    layout: WorksheetLayout,
}

#[derive(Clone, Copy)]
struct LayoutChange {
    rows: bool,
    columns: bool,
}

#[derive(Clone, Default)]
pub struct Worksheet {
    pub revision: Option<u64>,
    pub number: usize,
    pub table: Option<Table>,
    pub available: bool,
    pub is_markdown: bool,
    pub anchor: (usize, usize),
    pub selected: (usize, usize),
    pub selection_kind: SelectionKind,
    pub selected_rows: BTreeSet<usize>,
    pub selected_columns: BTreeSet<usize>,
    pub row_drag: Option<RowDrag>,
    pub resize_drag: Option<ResizeDrag>,
    pub layout: WorksheetLayout,
    selection_history: Arc<HashMap<u64, SelectionSnapshot>>,
    layout_edits: Arc<HashMap<(u64, u64), LayoutChange>>,
    history_state: Option<u64>,
    pub editing: bool,
    pub focus: bool,
    pub text_selection: Option<Range>,
    pub keyboard_focus: bool,
    pub reveal: bool,
    pub scroll: (f32, f32),
}
impl Worksheet {
    fn selection_snapshot(&self) -> SelectionSnapshot {
        SelectionSnapshot {
            kind: self.selection_kind,
            anchor: self.anchor,
            selected: self.selected,
            rows: self.selected_rows.clone(),
            columns: self.selected_columns.clone(),
            layout: self.layout.clone(),
        }
    }
    fn restore_selection(&mut self, state: u64) {
        if let Some(snapshot) = self.selection_history.get(&state).cloned() {
            let changed = self.selection_kind != snapshot.kind
                || self.selected != snapshot.selected
                || self.selected_rows != snapshot.rows
                || self.selected_columns != snapshot.columns;
            self.selection_kind = snapshot.kind;
            self.anchor = snapshot.anchor;
            self.selected = snapshot.selected;
            self.selected_rows = snapshot.rows;
            self.selected_columns = snapshot.columns;
            if let Some(change) = self
                .history_state
                .and_then(|before| self.layout_edits.get(&(before, state)))
            {
                if change.rows {
                    self.layout.row_heights = snapshot.layout.row_heights;
                }
                if change.columns {
                    self.layout.column_widths = snapshot.layout.column_widths;
                }
            }
            self.reveal |= changed;
        }
    }
    pub fn column_width(&self, column: usize) -> f32 {
        self.layout
            .column_widths
            .get(column)
            .copied()
            .unwrap_or(DEFAULT_COLUMN_WIDTH)
    }
    pub fn row_height(&self, row: usize) -> f32 {
        self.layout
            .row_heights
            .get(row)
            .copied()
            .unwrap_or(DEFAULT_ROW_HEIGHT)
    }
    pub fn bounds(&self) -> (usize, usize, usize, usize) {
        if let Some(table) = &self.table {
            match self.selection_kind {
                SelectionKind::Rows if !self.selected_rows.is_empty() => {
                    return (
                        *self.selected_rows.first().expect("nonempty"),
                        *self.selected_rows.last().expect("nonempty"),
                        0,
                        table.aligns.len() - 1,
                    );
                }
                SelectionKind::Columns if !self.selected_columns.is_empty() => {
                    return (
                        0,
                        table.rows.len() - 1,
                        *self.selected_columns.first().expect("nonempty"),
                        *self.selected_columns.last().expect("nonempty"),
                    );
                }
                _ => {}
            }
        }
        (
            self.anchor.0.min(self.selected.0),
            self.anchor.0.max(self.selected.0),
            self.anchor.1.min(self.selected.1),
            self.anchor.1.max(self.selected.1),
        )
    }

    pub fn rows(&self) -> Vec<usize> {
        if self.selection_kind == SelectionKind::Rows {
            self.selected_rows.iter().copied().collect()
        } else {
            let (a, b, _, _) = self.bounds();
            (a..=b).collect()
        }
    }
    pub fn columns(&self) -> Vec<usize> {
        if self.selection_kind == SelectionKind::Columns {
            self.selected_columns.iter().copied().collect()
        } else {
            let (_, _, a, b) = self.bounds();
            (a..=b).collect()
        }
    }
    pub fn contains(&self, row: usize, column: usize) -> bool {
        match self.selection_kind {
            SelectionKind::Rows => self.selected_rows.contains(&row),
            SelectionKind::Columns => self.selected_columns.contains(&column),
            SelectionKind::Cells => {
                let (a, b, c, d) = self.bounds();
                (a..=b).contains(&row) && (c..=d).contains(&column)
            }
        }
    }
    fn select_axis(&mut self, kind: SelectionKind, index: usize, extend: bool, toggle: bool) {
        let same = self.selection_kind == kind;
        let (anchor, selected) = if kind == SelectionKind::Rows {
            (&mut self.anchor.0, &mut self.selected_rows)
        } else {
            (&mut self.anchor.1, &mut self.selected_columns)
        };
        if extend && same {
            if !toggle {
                selected.clear();
            }
            selected.extend((*anchor).min(index)..=(*anchor).max(index));
        } else if toggle && same {
            if !selected.remove(&index) {
                selected.insert(index);
            }
            if selected.is_empty() {
                selected.insert(index);
            }
            *anchor = index;
        } else {
            *selected = BTreeSet::from([index]);
            *anchor = index;
        }
        let primary = if selected.contains(&index) {
            index
        } else {
            *selected.last().expect("nonempty selection")
        };
        if !extend {
            *anchor = primary;
        }
        self.selected = if kind == SelectionKind::Rows {
            (primary, 0)
        } else {
            (0, primary)
        };
        self.selection_kind = kind;
        self.editing = false;
        self.text_selection = None;
        self.focus = true;
    }
}
#[derive(Default)]
pub struct Worksheets {
    pub views: HashMap<ViewId, Worksheet>,
}

impl AppCore {
    pub fn worksheet_prepare(&mut self, view: ViewId) {
        self.worksheets
            .views
            .retain(|v, _| self.workbench.view(*v).is_some());
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(binding) = &v.markdown_table else {
            return;
        };
        let Some(doc) = self.workbench.document(v.document_id) else {
            return;
        };
        let state = self.worksheets.views.entry(view).or_default();
        let is_markdown = doc.language.as_str() == "markdown";
        if state.revision == Some(doc.revision) && state.is_markdown == is_markdown {
            return;
        }
        state.is_markdown = is_markdown;
        state.row_drag = None;
        state.resize_drag = None;
        state.available = false;
        let history_state = doc.buffer.state();
        if state.revision != Some(doc.revision) {
            state.restore_selection(history_state);
        }
        let mut resolved_range = None;
        if doc.language.as_str() == "markdown" && binding.range.0 < binding.range.1 {
            let tables = markdown_table::tables(&doc.buffer.text());
            if let Some((number, table)) = tables
                .into_iter()
                .enumerate()
                .find(|(_, t)| t.range.0 == binding.range.0)
            {
                state.selected.0 = state.selected.0.min(table.rows.len() - 1);
                state.selected.1 = state.selected.1.min(table.aligns.len() - 1);
                state.anchor.0 = state.anchor.0.min(table.rows.len() - 1);
                state.anchor.1 = state.anchor.1.min(table.aligns.len() - 1);
                state.selected_rows.retain(|r| *r < table.rows.len());
                state.selected_columns.retain(|c| *c < table.aligns.len());
                if state.selection_kind == SelectionKind::Rows && state.selected_rows.is_empty() {
                    state.selected_rows.insert(state.selected.0);
                }
                if state.selection_kind == SelectionKind::Columns
                    && state.selected_columns.is_empty()
                {
                    state.selected_columns.insert(state.selected.1);
                }
                state.number = number + 1;
                state.layout.resize(table.rows.len(), table.aligns.len());
                resolved_range = Some(table.range);
                state.table = Some(table);
                state.available = true;
            }
        }
        if !state.available {
            state.editing = false;
        }
        state.revision = Some(doc.revision);
        state.history_state = Some(history_state);
        if let Some(range) = resolved_range
            && let Some(binding) = self
                .workbench
                .views
                .get_mut(view)
                .and_then(|v| v.markdown_table.as_mut())
        {
            binding.range = range;
            binding.checkpoints.insert(history_state, range);
        }
    }

    pub fn worksheet_open(&mut self, inv: &CommandInvocation) {
        let Some(source) = inv
            .args
            .get("view")
            .and_then(Value::as_u64)
            .map(ViewId)
            .or(self.workbench.active_view())
        else {
            return;
        };
        let Some(v) = self.workbench.view(source) else {
            return;
        };
        let doc_id = v.document_id;
        let Some(doc) = self.workbench.document(doc_id) else {
            return;
        };
        if doc.language.as_str() != "markdown" {
            return;
        }
        let revision = doc.revision;
        if inv
            .args
            .get("revision")
            .and_then(Value::as_u64)
            .is_some_and(|r| r != revision)
        {
            return;
        }
        let at = inv
            .args
            .get("start")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(v.selection.primary().head);
        let tables = markdown_table::tables(&doc.buffer.text());
        let Some(table) = tables.iter().find(|t| t.range.0 <= at && at <= t.range.1) else {
            return;
        };
        let range = table.range;
        let existing = self
            .workbench
            .views
            .iter()
            .find(|v| {
                v.document_id == doc_id
                    && v.kind == ViewKind::MarkdownTable
                    && v.markdown_table.as_ref().is_some_and(|t| t.range == range)
            })
            .map(|v| v.id);
        if let Some(v) = existing {
            let _ = self.workbench.activate_view(v);
            return;
        }
        let state = doc.buffer.state();
        let group = self
            .workbench
            .layout
            .group_of(source)
            .unwrap_or(self.workbench.layout.active_group);
        if let Ok(view) = self
            .workbench
            .open_view(doc_id, group, TabPosition::AfterActive)
        {
            let v = self.workbench.views.get_mut(view).expect("opened");
            v.kind = ViewKind::MarkdownTable;
            v.markdown_table = Some(TableBinding {
                id: view,
                range,
                checkpoints: HashMap::from([(state, range)]),
            });
            v.preview = false;
            self.worksheet_prepare(view);
        }
    }

    pub fn worksheet_action(&mut self, inv: &CommandInvocation) {
        let Some(view) = inv
            .args
            .get("view")
            .and_then(Value::as_u64)
            .map(ViewId)
            .or(self.workbench.active_view())
        else {
            return;
        };
        if !self
            .workbench
            .view(view)
            .is_some_and(|v| v.kind == ViewKind::MarkdownTable)
        {
            return;
        }
        self.worksheet_prepare(view);
        let op = inv.args.get("op").and_then(Value::as_str).unwrap_or("");
        if matches!(op, "resizeRow" | "resizeColumn") {
            self.worksheet_resize(view, inv, op == "resizeRow");
            return;
        }
        let Some(mut s) = self.worksheets.views.get(&view).cloned() else {
            return;
        };
        let doc_id = self.workbench.view(view).expect("exists").document_id;
        if op == "source" {
            let at = self
                .workbench
                .view(view)
                .and_then(|v| v.markdown_table.as_ref())
                .map_or(0, |t| t.range.0);
            let existing = self
                .workbench
                .views
                .iter()
                .find(|v| v.document_id == doc_id && v.kind != ViewKind::MarkdownTable)
                .map(|v| v.id);
            let target = existing.or_else(|| {
                self.workbench
                    .open_view(
                        doc_id,
                        self.workbench.layout.active_group,
                        TabPosition::AfterActive,
                    )
                    .ok()
            });
            if let Some(v) = target {
                if let Some(v) = self.workbench.views.get_mut(v) {
                    v.kind = ViewKind::Text;
                    v.selection = Selection::caret(at);
                }
                let _ = self.workbench.activate_view(v);
                self.effects.push(Effect::FocusEditor(v));
            }
            return;
        }
        if op == "finish" {
            s.editing = false;
            self.workbench
                .document_mut(doc_id)
                .expect("exists")
                .buffer
                .seal();
            self.worksheets.views.insert(view, s);
            return;
        }
        if !s.available {
            return;
        }
        let Some(table) = s.table.clone() else { return };
        let doc = self.workbench.document(doc_id).expect("exists");
        let read_only = doc.is_read_only();
        let revision = doc.revision;
        let before_state = doc.buffer.state();
        let before_selection = s.selection_snapshot();
        if inv
            .args
            .get("revision")
            .and_then(Value::as_u64)
            .is_some_and(|r| r != revision)
        {
            return;
        }
        let usize_arg = |key: &str, fallback| {
            inv.args
                .get(key)
                .and_then(Value::as_u64)
                .map_or(fallback, |n| n as usize)
        };
        let value = inv.args.get("value").and_then(Value::as_str).unwrap_or("");
        let (r, c) = s.selected;
        let (r0, r1, c0, c1) = s.bounds();
        let selected_rows = s.rows();
        let selected_columns = s.columns();
        let mut values = table.values();
        let mut aligns = table.aligns.clone();
        let mut changes = None;
        let mut kind = EditKind::Other;
        let mut selection = Selection::caret(table.rows[r][c].range.0);
        match op {
            "selectRows" | "selectColumns" => {
                let kind = if op == "selectRows" {
                    SelectionKind::Rows
                } else {
                    SelectionKind::Columns
                };
                let index = if kind == SelectionKind::Rows {
                    usize_arg("row", r).min(values.len() - 1)
                } else {
                    usize_arg("column", c).min(aligns.len() - 1)
                };
                s.select_axis(
                    kind,
                    index,
                    inv.args
                        .get("extend")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    inv.args
                        .get("toggle")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
                self.workbench
                    .document_mut(doc_id)
                    .expect("exists")
                    .buffer
                    .seal();
            }
            "select" | "edit" => {
                s.selection_kind = SelectionKind::Cells;
                s.selected_rows.clear();
                s.selected_columns.clear();
                s.selected = (
                    usize_arg("row", r).min(values.len() - 1),
                    usize_arg("column", c).min(aligns.len() - 1),
                );
                if !inv
                    .args
                    .get("extend")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    s.anchor = s.selected;
                }
                s.editing = op == "edit" && !read_only;
                s.focus = s.editing;
                s.text_selection = None;
                s.reveal = inv
                    .args
                    .get("reveal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.workbench
                    .document_mut(doc_id)
                    .expect("exists")
                    .buffer
                    .seal();
            }
            "all" => {
                s.selection_kind = SelectionKind::Cells;
                s.anchor = (0, 0);
                s.selected = (values.len() - 1, aligns.len() - 1);
            }
            "copy" | "cut" => {
                if s.editing {
                    let raw = &values[r][c];
                    let range = s
                        .text_selection
                        .unwrap_or(Range::new(0, raw.chars().count()));
                    let a = range.from().min(raw.chars().count());
                    let b = range.to().min(raw.chars().count());
                    self.effects
                        .push(Effect::CopyText(raw.chars().skip(a).take(b - a).collect()));
                    if op == "cut" && !read_only {
                        let cell: String = raw.chars().take(a).chain(raw.chars().skip(b)).collect();
                        changes = table.cell_edit(r, c, &cell);
                    }
                } else {
                    let copied = selected_rows
                        .iter()
                        .map(|&r| {
                            selected_columns
                                .iter()
                                .map(|&c| values[r][c].as_str())
                                .collect::<Vec<_>>()
                                .join("\t")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.effects.push(Effect::CopyText(copied));
                    if op == "cut" && !read_only {
                        for &r in &selected_rows {
                            for &c in &selected_columns {
                                values[r][c].clear();
                            }
                        }
                        changes = table.replace(&values, &aligns);
                    }
                }
            }
            "set" if !read_only => {
                changes = table.cell_edit(r, c, value);
                let next = markdown_table::safe_cell(value);
                let old = &table.rows[r][c].markdown;
                let p = old
                    .chars()
                    .zip(next.chars())
                    .take_while(|(a, b)| a == b)
                    .count();
                let suffix = old
                    .chars()
                    .rev()
                    .zip(next.chars().rev())
                    .take(old.chars().count().min(next.chars().count()) - p)
                    .take_while(|(a, b)| a == b)
                    .count();
                // Minimal character edit keeps typing coalescence and anchors stable.
                if changes.as_ref().is_some_and(|cs| {
                    cs.edits().len() == 1
                        && cs.edits()[0].start == table.rows[r][c].range.0
                        && cs.edits()[0].end == table.rows[r][c].range.1
                }) {
                    let start = table.rows[r][c].range.0 + p;
                    let end = table.rows[r][c].range.1 - suffix;
                    let inserted: String = next
                        .chars()
                        .skip(p)
                        .take(next.chars().count() - p - suffix)
                        .collect();
                    changes = Some(ChangeSet::replace(start, end, inserted.clone()));
                    kind = if start == end {
                        EditKind::Insert
                    } else if inserted.is_empty() {
                        EditKind::DeleteBackward
                    } else {
                        EditKind::Other
                    };
                    selection = Selection::caret(start + inserted.chars().count());
                }
            }
            "clear" if !read_only => {
                for &r in &selected_rows {
                    for &c in &selected_columns {
                        values[r][c].clear();
                    }
                }
                changes = table.replace(&values, &aligns);
            }
            "paste" if !read_only => {
                if s.editing {
                    let raw = &values[r][c];
                    let range = s
                        .text_selection
                        .unwrap_or(Range::caret(raw.chars().count()));
                    let a = range.from().min(raw.chars().count());
                    let b = range.to().min(raw.chars().count());
                    let mut cell: String = raw.chars().take(a).collect();
                    cell.push_str(&markdown_table::safe_cell(value));
                    cell.extend(raw.chars().skip(b));
                    changes = table.cell_edit(r, c, &cell);
                } else {
                    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
                    let pasted: Vec<Vec<_>> = normalized
                        .strip_suffix('\n')
                        .unwrap_or(&normalized)
                        .split('\n')
                        .map(|row| row.split('\t').map(str::to_owned).collect())
                        .collect();
                    let width = aligns
                        .len()
                        .max(c0 + pasted.iter().map(Vec::len).max().unwrap_or(1));
                    let previous_width = aligns.len();
                    aligns.resize(width, Align::None);
                    for row in &mut values {
                        // GFM ignores surplus source cells. Keep them after
                        // newly materialized columns rather than truncating.
                        row.splice(
                            previous_width..previous_width,
                            std::iter::repeat_n(String::new(), width - previous_width),
                        );
                    }
                    values.resize(
                        r0 + pasted.len().max(values.len().saturating_sub(r0)),
                        vec![String::new(); width],
                    );
                    for (i, row) in pasted.iter().enumerate() {
                        for (j, value) in row.iter().enumerate() {
                            values[r0 + i][c0 + j] = value.clone();
                        }
                    }
                    changes = table.replace(&values, &aligns);
                    s.layout.resize(values.len(), aligns.len());
                    s.selection_kind = SelectionKind::Cells;
                    s.anchor = (r0, c0);
                    s.selected = (
                        r0 + pasted.len() - 1,
                        c0 + pasted.iter().map(Vec::len).max().unwrap_or(1) - 1,
                    );
                }
            }
            "rowBefore" | "rowAfter" if !read_only && (op != "rowBefore" || r0 > 0) => {
                let at = if op == "rowBefore" { r0.max(1) } else { r1 + 1 };
                values.insert(at, vec![String::new(); aligns.len()]);
                Arc::make_mut(&mut s.layout.row_heights).insert(at, DEFAULT_ROW_HEIGHT);
                changes = table.replace(&values, &aligns);
                if s.selection_kind == SelectionKind::Rows {
                    s.select_axis(SelectionKind::Rows, at, false, false);
                }
            }
            "deleteRows" if !read_only && selected_rows.iter().any(|r| *r > 0) => {
                for &row in selected_rows.iter().rev().filter(|&&r| r > 0) {
                    values.remove(row);
                    Arc::make_mut(&mut s.layout.row_heights).remove(row);
                }
                changes = table.replace(&values, &aligns);
                if s.selection_kind == SelectionKind::Rows {
                    s.select_axis(SelectionKind::Rows, r0.min(values.len() - 1), false, false);
                }
            }
            "columnBefore" | "columnAfter" if !read_only => {
                let at = if op == "columnAfter" { c1 + 1 } else { c0 };
                aligns.insert(at, Align::None);
                Arc::make_mut(&mut s.layout.column_widths).insert(at, DEFAULT_COLUMN_WIDTH);
                for row in &mut values {
                    row.insert(at, String::new());
                }
                changes = table.replace(&values, &aligns);
                if s.selection_kind == SelectionKind::Columns {
                    s.select_axis(SelectionKind::Columns, at, false, false);
                }
            }
            "deleteColumns" if !read_only && selected_columns.len() < aligns.len() => {
                for &column in selected_columns.iter().rev() {
                    aligns.remove(column);
                    Arc::make_mut(&mut s.layout.column_widths).remove(column);
                }
                for row in &mut values {
                    for &column in selected_columns.iter().rev() {
                        row.remove(column);
                    }
                }
                changes = table.replace(&values, &aligns);
                if s.selection_kind == SelectionKind::Columns {
                    s.select_axis(
                        SelectionKind::Columns,
                        c0.min(aligns.len() - 1),
                        false,
                        false,
                    );
                }
            }
            "moveRows" if !read_only => {
                let rows = inv
                    .args
                    .get("rows")
                    .and_then(Value::as_array)
                    .and_then(|rows| {
                        rows.iter()
                            .map(|r| r.as_u64().map(|n| n as usize))
                            .collect::<Option<Vec<_>>>()
                    });
                if let Some(rows) = rows
                    && let Some((edit, moved)) =
                        table.move_rows(&rows, usize_arg("before", values.len()))
                {
                    let chosen: BTreeSet<_> = rows.iter().copied().collect();
                    let moving: Vec<_> = chosen.iter().map(|&r| s.row_height(r)).collect();
                    let mut heights: Vec<_> = s
                        .layout
                        .row_heights
                        .iter()
                        .enumerate()
                        .filter(|(r, _)| !chosen.contains(r))
                        .map(|(_, h)| *h)
                        .collect();
                    heights.splice(moved[0]..moved[0], moving);
                    s.layout.row_heights = Arc::new(heights);
                    changes = Some(edit);
                    s.selection_kind = SelectionKind::Rows;
                    s.selected = (moved[0], 0);
                    s.anchor = s.selected;
                    s.selected_rows = moved.into_iter().collect();
                    s.editing = false;
                    s.row_drag = None;
                    s.focus = true;
                    s.reveal = true;
                }
            }
            "align" if !read_only => {
                let a = match value {
                    "left" => Align::Left,
                    "center" => Align::Center,
                    "right" => Align::Right,
                    _ => Align::None,
                };
                for &c in &selected_columns {
                    aligns[c] = a;
                }
                changes = table.replace(&values, &aligns);
            }
            "link" if !read_only => {
                self.open_md_insert_dialog(view, crate::markdown_insert::InsertDialogKind::Link);
                return;
            }
            "format" if !read_only => {
                let marker = match value {
                    "bold" => "**",
                    "italic" => "*",
                    "strike" => "~~",
                    "code" => "`",
                    _ => "",
                };
                if s.editing {
                    let raw = &values[r][c];
                    let range = s
                        .text_selection
                        .unwrap_or(Range::new(0, raw.chars().count()));
                    let rope = notoir_editor::Rope::from_str(raw);
                    let (mut a, mut b) = if range.is_empty() {
                        notoir_editor::line_ops::word_at(&rope, range.head.min(rope.len_chars()))
                    } else {
                        (
                            range.from().min(rope.len_chars()),
                            range.to().min(rope.len_chars()),
                        )
                    };
                    let n = marker.chars().count();
                    // Include surrounding markers when toggling a selected
                    // word that is already formatted.
                    if op == "format"
                        && n > 0
                        && a >= n
                        && b + n <= rope.len_chars()
                        && rope.slice(a - n..a) == marker
                        && rope.slice(b..b + n) == marker
                        && (marker != "*"
                            || a < 2
                            || rope.char(a - 2) != '*'
                            || (a >= 3 && rope.char(a - 3) == '*'))
                    {
                        a -= n;
                        b += n;
                    }
                    let formatted = format_cell(&rope.slice(a..b).to_string(), marker);
                    let mut cell = raw.chars().take(a).collect::<String>();
                    cell.push_str(&formatted);
                    cell.extend(raw.chars().skip(b));
                    changes = table.cell_edit(r, c, &cell);
                } else {
                    for &r in &selected_rows {
                        for &c in &selected_columns {
                            values[r][c] = format_cell(&values[r][c], marker);
                        }
                    }
                    changes = table.replace(&values, &aligns);
                }
                s.focus = s.editing;
            }
            _ => {}
        }
        self.worksheets.views.insert(view, s);
        if let Some(changes) = changes.filter(|cs| !cs.is_empty()) {
            if kind == EditKind::Other {
                self.workbench
                    .document_mut(doc_id)
                    .expect("exists")
                    .buffer
                    .seal();
            }
            let edited = self
                .workbench
                .edit(view, changes, selection, kind, self.now_ms);
            if edited.is_ok() {
                let after_state = self
                    .workbench
                    .document(doc_id)
                    .expect("exists")
                    .buffer
                    .state();
                if let Some(state) = self.worksheets.views.get_mut(&view) {
                    let after_selection = state.selection_snapshot();
                    let history = Arc::make_mut(&mut state.selection_history);
                    history.insert(before_state, before_selection);
                    history.insert(after_state, after_selection);
                    let layout_change = LayoutChange {
                        rows: matches!(op, "rowBefore" | "rowAfter" | "deleteRows" | "moveRows")
                            || values.len() != table.rows.len(),
                        columns: matches!(op, "columnBefore" | "columnAfter" | "deleteColumns")
                            || aligns.len() != table.aligns.len(),
                    };
                    if layout_change.rows || layout_change.columns {
                        let layout_edits = Arc::make_mut(&mut state.layout_edits);
                        layout_edits.insert((before_state, after_state), layout_change);
                        layout_edits.insert((after_state, before_state), layout_change);
                    }
                }
            }
            self.worksheet_prepare(view);
        }
    }

    fn worksheet_resize(&mut self, view: ViewId, inv: &CommandInvocation, row: bool) {
        let Some(doc) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            return;
        };
        let revision = doc.revision;
        let history_state = doc.buffer.state();
        if inv
            .args
            .get("revision")
            .and_then(Value::as_u64)
            .is_some_and(|r| r != revision)
        {
            return;
        }
        let Some(index) = inv
            .args
            .get(if row { "row" } else { "column" })
            .and_then(Value::as_u64)
            .map(|n| n as usize)
        else {
            return;
        };
        let Some(size) = inv
            .args
            .get("size")
            .and_then(Value::as_f64)
            .map(|n| n as f32)
            .filter(|n| n.is_finite())
        else {
            return;
        };
        let Some(state) = self.worksheets.views.get_mut(&view).filter(|s| s.available) else {
            return;
        };
        let sizes = if row {
            &mut state.layout.row_heights
        } else {
            &mut state.layout.column_widths
        };
        let Some(value) = Arc::make_mut(sizes).get_mut(index) else {
            return;
        };
        *value = (if row {
            ResizeAxis::Row
        } else {
            ResizeAxis::Column
        })
        .constrain(size);
        state.row_drag = None;
        self.worksheet_record_selection(view, history_state);
    }

    pub(crate) fn worksheet_link_target(
        &mut self,
        view: ViewId,
    ) -> Option<(crate::markdown_insert::WorksheetLinkTarget, String)> {
        self.worksheet_prepare(view);
        let state = self.worksheets.views.get(&view)?;
        if !state.available {
            return None;
        }
        let table = state.table.as_ref()?;
        let binding = self.workbench.view(view)?.markdown_table.as_ref()?;
        let (r, c) = state.selected;
        let cell = table.rows.get(r)?.get(c)?;
        let raw = &cell.markdown;
        let mut text = state.editing.then(|| {
            let range = state
                .text_selection
                .unwrap_or(Range::caret(raw.chars().count()));
            Range::new(
                range.anchor.min(raw.chars().count()),
                range.head.min(raw.chars().count()),
            )
        });
        let bounds = if state.editing {
            (r, r, c, c)
        } else {
            state.bounds()
        };
        let link = if bounds.0 == bounds.1 && bounds.2 == bounds.3 {
            // Parse in document context so reference-style links resolve too.
            let selection = text.unwrap_or(Range::new(0, raw.chars().count()));
            let document = self.workbench.view(view)?.document_id;
            let source = self.workbench.document(document)?.buffer.text();
            notoir_editor::markdown_edit::existing_link(
                &source,
                Range::new(
                    cell.range.0 + selection.from(),
                    cell.range.0 + selection.to(),
                ),
            )
            .filter(|link| link.range.from() >= cell.range.0 && link.range.to() <= cell.range.1)
            .map(|mut link| {
                link.range = Range::new(
                    link.range.from() - cell.range.0,
                    link.range.to() - cell.range.0,
                );
                link
            })
        } else {
            None
        };
        let caption = if let Some(link) = &link {
            text = Some(link.range);
            link.caption.clone()
        } else {
            match text {
                Some(range) => raw
                    .chars()
                    .skip(range.from())
                    .take(range.to() - range.from())
                    .collect(),
                None => raw.clone(),
            }
        };
        Some((
            crate::markdown_insert::WorksheetLinkTarget {
                table: binding.id,
                range: binding.range,
                bounds,
                rows: state.rows(),
                columns: state.columns(),
                text,
                link,
            },
            caption,
        ))
    }

    pub(crate) fn worksheet_link_edit(
        &mut self,
        view: ViewId,
        target: &crate::markdown_insert::WorksheetLinkTarget,
        caption: &str,
        destination: &str,
    ) -> Option<notoir_editor::edit_ops::EditResult> {
        self.worksheet_prepare(view);
        let state = self.worksheets.views.get(&view)?;
        if !state.available {
            return None;
        }
        let table = state.table.as_ref()?;
        let (r0, r1, c0, c1) = target.bounds;
        let link = if let Some(link) = &target.link {
            notoir_editor::markdown_edit::update_link(link, caption, destination)
        } else {
            notoir_editor::markdown_edit::link(Range::caret(0), caption, destination, false)
        };
        let link = link.changes.edits().first()?.text.to_string();
        let (changes, caret) = if r0 == r1 && c0 == c1 {
            let cell = table.rows.get(r0)?.get(c0)?;
            let value = if let Some(range) = target.text {
                let mut value: String = cell.markdown.chars().take(range.from()).collect();
                value.push_str(&link);
                value.extend(cell.markdown.chars().skip(range.to()));
                value
            } else {
                link
            };
            (
                table.cell_edit(r0, c0, &value)?,
                cell.range.0 + markdown_table::safe_cell(&value).chars().count(),
            )
        } else {
            let mut values = table.values();
            for &r in &target.rows {
                let row = values.get_mut(r)?;
                for &c in &target.columns {
                    *row.get_mut(c)? = link.clone();
                }
            }
            (table.replace(&values, &table.aligns)?, table.range.0)
        };
        Some(notoir_editor::edit_ops::EditResult {
            changes,
            selection: Selection::caret(caret),
            kind: EditKind::Other,
        })
    }

    pub fn worksheet_clipboard(&mut self, op: &str, value: Option<&str>) -> bool {
        let Some(view) = self.workbench.active_view().filter(|v| {
            self.workbench
                .view(*v)
                .is_some_and(|v| v.kind == ViewKind::MarkdownTable)
        }) else {
            return false;
        };
        let inv = CommandInvocation::new(
            "markdown.tableAction",
            notoir_core::command::InvocationSource::Internal,
        )
        .with_args(json!({"view":view.0,"op":op,"value":value.unwrap_or("")}));
        self.worksheet_action(&inv);
        true
    }

    pub(crate) fn worksheet_record_selection(&mut self, view: ViewId, history_state: u64) {
        if let Some(state) = self.worksheets.views.get_mut(&view) {
            let snapshot = state.selection_snapshot();
            Arc::make_mut(&mut state.selection_history).insert(history_state, snapshot);
        }
    }
}

fn format_cell(raw: &str, marker: &str) -> String {
    if marker.is_empty() {
        return raw.into();
    }
    if marker == "`" && raw.starts_with('`') {
        let fence = raw.chars().take_while(|c| *c == '`').count();
        if raw.len() >= fence * 2 && raw.chars().rev().take_while(|c| *c == '`').count() == fence {
            let inner = &raw[fence..raw.len() - fence];
            return if inner.starts_with(' ') && inner.ends_with(' ') && !inner.trim().is_empty() {
                inner[1..inner.len() - 1].into()
            } else {
                inner.into()
            };
        }
    }
    let marker_is_style = marker != "*" || !raw.starts_with("**") || raw.starts_with("***");
    if marker_is_style
        && raw.len() >= marker.len() * 2
        && raw.starts_with(marker)
        && raw.ends_with(marker)
    {
        return raw[marker.len()..raw.len() - marker.len()].into();
    }
    let marker = if marker == "`" {
        "`".repeat(raw.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1)
    } else {
        marker.into()
    };
    let padding = if marker.starts_with('`') && (raw.starts_with('`') || raw.ends_with('`')) {
        " "
    } else {
        ""
    };
    format!("{marker}{padding}{raw}{padding}{marker}")
}
