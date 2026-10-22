//! CSV presentation state and commands; the text buffer remains authoritative.
use crate::{core::AppCore, io::IoResult};
use notoir_core::ids::ViewId;
use notoir_core::view::ViewKind;
use notoir_editor::{EditKind, Selection, csv};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const MAX_BYTES: usize = 20 * 1024 * 1024;
#[derive(Clone)]
pub struct Editing {
    pub row: usize,
    pub column: usize,
    pub revision: u64,
    pub value: String,
    pub focus: bool,
}
#[derive(Clone)]
pub struct CsvView {
    pub query_generation: u64,
    pub query_applied: Option<u64>,
    pub query_pending: Option<u64>,
    pub rows: Vec<usize>,
    pub found: Vec<(usize, usize)>,
    pub header: bool,
    pub separator: Option<char>,
    pub detected: char,
    pub hidden: HashSet<usize>,
    pub filters: HashMap<usize, String>,
    pub search: String,
    pub selected_match: usize,
    pub scroll_match: bool,
    pub editing: Option<Editing>,
    pub parsed: Option<Arc<csv::Table>>,
    pub revision: Option<u64>,
    pub pending: Option<(u64, Option<char>)>,
    pub error: Option<csv::Error>,
    pub too_large: bool,
}
impl Default for CsvView {
    fn default() -> Self {
        Self {
            query_generation: 0,
            query_applied: None,
            query_pending: None,
            rows: Vec::new(),
            found: Vec::new(),
            header: true,
            separator: None,
            detected: ',',
            hidden: HashSet::new(),
            filters: HashMap::new(),
            search: String::new(),
            selected_match: 0,
            scroll_match: false,
            editing: None,
            parsed: None,
            revision: None,
            pending: None,
            error: None,
            too_large: false,
        }
    }
}
#[derive(Default)]
pub struct CsvState {
    pub views: HashMap<ViewId, CsvView>,
}
#[derive(Debug)]
pub struct Parsed {
    pub view: ViewId,
    pub revision: u64,
    pub requested_separator: Option<char>,
    pub separator: char,
    pub result: Result<csv::Table, csv::Error>,
}
impl CsvView {
    pub fn visible_rows(&self, table: &csv::Table) -> Vec<usize> {
        table
            .rows
            .iter()
            .enumerate()
            .skip(usize::from(self.header))
            .filter_map(|(r, cells)| {
                self.filters
                    .iter()
                    .all(|(&c, q)| {
                        cells
                            .get(c)
                            .map_or("", |c| c.value.as_str())
                            .to_lowercase()
                            .contains(&q.to_lowercase())
                    })
                    .then_some(r)
            })
            .collect()
    }
    pub fn matches(&self, table: &csv::Table, rows: &[usize]) -> Vec<(usize, usize)> {
        let q = self.search.to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        rows.iter()
            .flat_map(|&r| {
                let q = &q;
                (0..table.columns).filter_map(move |c| {
                    (!self.hidden.contains(&c)
                        && table.rows[r]
                            .get(c)
                            .is_some_and(|v| v.value.to_lowercase().contains(q.as_str())))
                    .then_some((r, c))
                })
            })
            .collect()
    }
}
impl AppCore {
    pub fn is_csv(&self, doc: notoir_core::ids::DocumentId) -> bool {
        self.workbench
            .document(doc)
            .is_some_and(|d| d.language.as_str() == "csv")
    }
    pub fn csv_prepare(&mut self, view: ViewId) {
        let Some(d) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            return;
        };
        let state = self.csv.views.entry(view).or_default();
        state.too_large = d.buffer.rope().len_bytes() > MAX_BYTES;
        if state.revision == Some(d.revision)
            && state.error.is_none()
            && state.query_applied != Some(state.query_generation)
            && state.query_pending.is_none()
            && let Some(table) = state.parsed.clone()
        {
            let generation = state.query_generation;
            let revision = d.revision;
            state.query_pending = Some(generation);
            let query = state.clone();
            self.fs.submit(move || {
                let rows = query.visible_rows(&table);
                let matches = query.matches(&table, &rows);
                Some(IoResult::CsvFiltered {
                    view,
                    generation,
                    revision,
                    rows,
                    matches,
                })
            });
        }
        if state.too_large || state.revision == Some(d.revision) || state.pending.is_some() {
            return;
        }
        let revision = d.revision;
        let text = d.buffer.text();
        let requested_separator = state.separator;
        state.pending = Some((revision, requested_separator));
        self.fs.submit(move || {
            let separator = requested_separator.unwrap_or_else(|| csv::detect(&text));
            Some(IoResult::CsvParsed(Box::new(Parsed {
                view,
                revision,
                requested_separator,
                separator,
                result: csv::parse(&text, separator),
            })))
        });
    }
    pub fn csv_parsed(&mut self, result: Parsed) {
        let Some(d) = self
            .workbench
            .view(result.view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            self.csv.views.remove(&result.view);
            return;
        };
        let state = self.csv.views.entry(result.view).or_default();
        if state.pending != Some((result.revision, result.requested_separator)) {
            return;
        }
        state.pending = None;
        if d.revision != result.revision || state.separator != result.requested_separator {
            return;
        }
        state.revision = Some(result.revision);
        state.detected = result.separator;
        match result.result {
            Ok(table) => {
                state.query_generation += 1;
                state.rows.clear();
                state.found.clear();
                state.parsed = Some(Arc::new(table));
                state.error = None;
            }
            Err(e) => {
                state.error = Some(e);
                state.editing = None;
            }
        }
    }
    pub fn csv_filtered(
        &mut self,
        view: ViewId,
        generation: u64,
        revision: u64,
        rows: Vec<usize>,
        matches: Vec<(usize, usize)>,
    ) {
        if let Some(state) = self.csv.views.get_mut(&view) {
            if state.query_pending == Some(generation) {
                state.query_pending = None;
            }
            if state.query_generation == generation && state.revision == Some(revision) {
                state.query_applied = Some(generation);
                state.rows = rows;
                state.found = matches;
            }
        }
    }
    pub fn csv_action(&mut self, inv: &notoir_core::command::CommandInvocation) {
        let Some(view) = inv
            .args
            .get("view")
            .and_then(|v| v.as_u64())
            .map(ViewId)
            .or_else(|| self.workbench.active_view())
        else {
            return;
        };
        let Some(doc) = self
            .workbench
            .view(view)
            .map(|v| v.document_id)
            .filter(|d| self.is_csv(*d))
        else {
            return;
        };
        let a = &inv.args;
        let op = a["op"].as_str().unwrap_or("");
        let s = self.csv.views.entry(view).or_default();
        if matches!(
            op,
            "header" | "separator" | "visible" | "showAll" | "filter" | "search"
        ) {
            s.query_generation += 1;
        }
        match op {
            "mode" => {
                if let Some(v) = self.workbench.views.get_mut(view) {
                    v.kind = match a["value"].as_str() {
                        Some("preview") => ViewKind::CsvPreview,
                        Some("split") => ViewKind::CsvSplit,
                        _ => ViewKind::Text,
                    };
                }
            }
            "header" => s.header = a["value"].as_bool().unwrap_or(true),
            "separator" => {
                s.separator = a["value"].as_str().and_then(|v| v.chars().next());
                s.revision = None;
                s.pending = None;
                s.parsed = None;
                s.error = None;
                s.editing = None;
            }
            "visible" => {
                if let Some(c) = a["column"].as_u64() {
                    if a["value"].as_bool() == Some(false) {
                        s.hidden.insert(c as usize);
                    } else {
                        s.hidden.remove(&(c as usize));
                    }
                }
            }
            "showAll" => s.hidden.clear(),
            "filter" => {
                if let Some(c) = a["column"].as_u64() {
                    let value = a["value"].as_str().unwrap_or("").to_owned();
                    if value.is_empty() {
                        s.filters.remove(&(c as usize));
                    } else {
                        s.filters.insert(c as usize, value);
                    }
                    s.selected_match = 0;
                }
            }
            "search" => {
                s.search = a["value"].as_str().unwrap_or("").to_owned();
                s.selected_match = 0;
                s.scroll_match = true;
            }
            "next" | "previous" => {
                if s.parsed.is_some() {
                    let n = s.found.len();
                    if n > 0 {
                        s.selected_match = if op == "next" {
                            (s.selected_match + 1) % n
                        } else {
                            (s.selected_match + n - 1) % n
                        };
                        s.scroll_match = true;
                    }
                }
            }
            "edit" => {
                if s.error.is_some() || s.pending.is_some() {
                    return;
                }
                if let (Some(row), Some(column), Some(d)) = (
                    a["row"].as_u64(),
                    a["column"].as_u64(),
                    self.workbench.document(doc),
                ) && s.revision == Some(d.revision)
                    && !d.is_read_only()
                    && s.parsed.as_ref().is_some_and(|t| {
                        (row as usize) < t.rows.len() && (column as usize) < t.columns
                    })
                {
                    s.editing = Some(Editing {
                        row: row as usize,
                        column: column as usize,
                        revision: d.revision,
                        value: s
                            .parsed
                            .as_ref()
                            .and_then(|t| t.rows.get(row as usize))
                            .and_then(|r| r.get(column as usize))
                            .map_or(String::new(), |c| c.value.clone()),
                        focus: true,
                    });
                }
            }
            "cancelEdit" => s.editing = None,
            "commit" => {
                let Some(editing) = s.editing.take() else {
                    return;
                };
                let Some(d) = self.workbench.document(doc) else {
                    return;
                };
                if d.revision != editing.revision
                    || s.revision != Some(d.revision)
                    || s.error.is_some()
                    || d.is_read_only()
                {
                    return;
                }
                let value = a["value"].as_str().unwrap_or(&editing.value);
                if let Some(t) = &s.parsed {
                    if t.rows
                        .get(editing.row)
                        .and_then(|r| r.get(editing.column))
                        .is_some_and(|c| c.value == value)
                    {
                        return;
                    }
                    if let Some(change) =
                        csv::edit(t, editing.row, editing.column, s.detected, value)
                    {
                        let selection = self
                            .workbench
                            .view(view)
                            .map_or(Selection::caret(0), |v| v.selection.map(&change));
                        let _ = self.workbench.edit(
                            view,
                            change,
                            selection,
                            EditKind::Other,
                            self.now_ms,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}
