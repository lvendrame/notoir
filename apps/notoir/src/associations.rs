//! Transient association tab. OS queries and changes never run on the UI thread.
use crate::{core::AppCore, io::IoResult, t};
use notoir_core::{
    command::{CommandInvocation, InvocationSource},
    ids::ViewId,
    view::ViewKind,
};
use notoir_platform::associations::{self as platform, Association, Backend, Outcome, Selection};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const TITLE: &str = "integration:associations.title";
#[derive(Default)]
pub struct State {
    pub backend: Option<Arc<dyn Backend>>,
    pub view: Option<ViewId>,
    pub generation: u64,
    pub busy: bool,
    pub loaded: bool,
    pub rows: Vec<Association>,
    pub desired: BTreeMap<String, bool>,
    pub dirty: BTreeSet<String>,
    pub errors: BTreeMap<String, String>,
    pub error: Option<String>,
    pub pending: bool,
    pub submitted: bool,
}
#[derive(Debug)]
pub struct Done {
    pub generation: u64,
    pub saving: bool,
    pub result: Result<Outcome, String>,
}
impl AppCore {
    pub fn open_associations(&mut self) {
        let existing = self
            .associations
            .view
            .filter(|v| self.workbench.view(*v).is_some());
        if let Some(v) = existing {
            let _ = self.workbench.activate_view(v);
            return;
        }
        self.associations.view = self.open_virtual(TITLE, ViewKind::FileAssociations);
        self.associations.rows.clear();
        self.associations.desired.clear();
        self.associations.dirty.clear();
        self.associations.loaded = false;
        self.associations.submitted = false;
        self.associations.pending = false;
        self.associations.busy = false;
        self.associations_request(false);
    }
    pub fn associations_request(&mut self, saving: bool) {
        if self.associations.busy {
            return;
        }
        let selections = self
            .associations
            .rows
            .iter()
            .filter_map(|row| {
                let selected = *self.associations.desired.get(&row.content_type)?;
                (selected != row.is_notoir
                    || (self.associations.dirty.contains(&row.content_type)
                        && !selected
                        && row.handler.is_none()))
                .then(|| Selection {
                    content_type: row.content_type.clone(),
                    expected_handler: row.handler.clone(),
                    selected,
                })
            })
            .collect::<BTreeMapByType>();
        if saving && !self.associations.loaded {
            return;
        }
        if saving && selections.0.is_empty() {
            self.close_associations();
            return;
        }
        self.associations.busy = true;
        self.associations.error = None;
        self.associations.errors.clear();
        self.associations.generation += 1;
        self.associations.submitted |= saving;
        let generation = self.associations.generation;
        let backend = self.associations.backend.clone();
        let journal = self.paths.data.join("file-associations-v1.json");
        self.io.submit(move || {
            let result = (|| {
                let backend = match backend {
                    Some(b) => b,
                    None => Arc::new(platform::NativeBackend::new()?) as Arc<dyn Backend>,
                };
                if saving {
                    platform::apply(backend.as_ref(), &journal, &selections.0)
                } else {
                    Ok(Outcome {
                        rows: platform::inspect(backend.as_ref())?,
                        ..Default::default()
                    })
                }
            })();
            Some(IoResult::Associations(Done {
                generation,
                saving,
                result,
            }))
        });
    }
    pub fn associations_done(&mut self, done: Done) {
        if done.generation != self.associations.generation
            || !self
                .associations
                .view
                .is_some_and(|v| self.workbench.view(v).is_some())
        {
            return;
        }
        self.associations.busy = false;
        match done.result {
            Err(e) => self.associations.error = Some(e),
            Ok(result) => {
                self.associations.rows = result.rows;
                self.associations.errors = result.errors;
                self.associations.loaded = true;
                for row in &self.associations.rows {
                    if !self.associations.dirty.contains(&row.content_type) {
                        self.associations
                            .desired
                            .insert(row.content_type.clone(), row.is_notoir);
                    }
                }
                let matches = self.associations.rows.iter().all(|r| {
                    self.associations.desired.get(&r.content_type) == Some(&r.is_notoir)
                        && (!self.associations.dirty.contains(&r.content_type)
                            || r.is_notoir
                            || r.handler.is_some())
                });
                self.associations.pending =
                    result.pending || (self.associations.submitted && !matches);
                if self.associations.submitted && matches && self.associations.errors.is_empty() {
                    self.close_associations();
                }
            }
        }
    }
    pub fn close_associations(&mut self) {
        self.associations.generation += 1;
        if let Some(view) = self.associations.view.take() {
            self.close_view_now(view);
        }
        self.associations.busy = false;
    }
    pub fn select_association(&mut self, content_type: &str, selected: bool) {
        if !self.associations.busy
            && self
                .associations
                .rows
                .iter()
                .any(|r| r.content_type == content_type)
        {
            self.associations
                .desired
                .insert(content_type.into(), selected);
            if self
                .associations
                .rows
                .iter()
                .any(|r| r.content_type == content_type && r.is_notoir != selected)
            {
                self.associations.dirty.insert(content_type.into());
            } else {
                self.associations.dirty.remove(content_type);
            }
        }
    }
}
// Collect one operation per OS content type, even when several checkboxes link.
struct BTreeMapByType(Vec<Selection>);
impl FromIterator<Selection> for BTreeMapByType {
    fn from_iter<T: IntoIterator<Item = Selection>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|s| (s.content_type.clone(), s))
                .collect::<BTreeMap<_, _>>()
                .into_values()
                .collect(),
        )
    }
}

fn enqueue(actions: &mut crate::shell::Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Menu).with_args(args));
}

fn extension_checkbox(
    ui: &mut egui::Ui,
    state: &State,
    row: &Association,
    actions: &mut crate::shell::Actions,
) {
    let mut selected = state
        .desired
        .get(&row.content_type)
        .copied()
        .unwrap_or(row.is_notoir);
    let response = ui.add_enabled(
        !state.busy,
        egui::Checkbox::new(&mut selected, format!(".{}", row.extension)),
    );
    crate::live::track(&response, &format!("associations.{}", row.extension));
    if response.changed() {
        enqueue(
            actions,
            "app.selectAssociation",
            serde_json::json!({"type":row.content_type,"selected":selected}),
        );
    }
    let linked: Vec<_> = state
        .rows
        .iter()
        .filter(|r| r.content_type == row.content_type)
        .map(|r| format!(".{}", r.extension))
        .collect();
    if linked.len() > 1 {
        response.on_hover_text(t!(
            "integration:associations.shared",
            extensions = linked.join(", ")
        ));
    }
    if let Some(error) = state.errors.get(&row.content_type) {
        ui.colored_label(
            ui.visuals().error_fg_color,
            t!("integration:associations.failed"),
        )
        .on_hover_text(error);
    } else if state.submitted
        && (selected != row.is_notoir
            || (state.dirty.contains(&row.content_type) && !selected && row.handler.is_none()))
    {
        ui.label(t!("integration:associations.pendingType"));
    }
}

pub fn show(ui: &mut egui::Ui, core: &AppCore, actions: &mut crate::shell::Actions) {
    let state = &core.associations;
    let focused = ui.input(|i| i.focused);
    let focus_id = ui.id().with("association-window-focus");
    let was_focused = ui.data_mut(|data| {
        let previous = data.get_temp::<bool>(focus_id).unwrap_or(focused);
        data.insert_temp(focus_id, focused);
        previous
    });
    if focused && !was_focused && state.pending && !state.busy {
        enqueue(actions, "app.refreshAssociations", serde_json::Value::Null);
    }
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.heading(t!("integration:associations.title"));
    });
    ui.add_space(10.0);
    ui.indent("association-content", |ui| {
        ui.label(t!("integration:associations.description"));
        ui.label(t!("integration:associations.linked"));
        if cfg!(windows) {
            ui.label(t!("integration:associations.windows"));
        }
        if state.submitted {
            ui.label(t!("integration:associations.partial"));
        }
        if state.pending {
            ui.label(t!("integration:associations.pending"));
        }
        if let Some(error) = &state.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                t!("integration:associations.failed"),
            );
            ui.label(error);
        }
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t!("integration:associations.working"));
            });
        }
        ui.add_space(12.0);
        let height = (ui.available_height() - 60.0).max(20.0);
        egui::ScrollArea::vertical()
            .id_salt("association-list")
            .max_height(height)
            .show(ui, |ui| {
                let columns = ((ui.available_width() / 210.0).floor() as usize).clamp(1, 3);
                let width = (ui.available_width() / columns as f32 - 16.0).max(90.0);
                egui::Grid::new("association-grid")
                    .num_columns(columns)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        for (index, row) in state.rows.iter().enumerate() {
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                extension_checkbox(ui, state, row, actions);
                            });
                            if (index + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            for (id, key, enabled) in [
                (
                    "app.saveAssociations",
                    "common:save",
                    !state.busy && state.loaded,
                ),
                (
                    "app.refreshAssociations",
                    "integration:associations.refresh",
                    !state.busy,
                ),
                ("app.cancelAssociations", "common:cancel", true),
            ] {
                let response = ui.add_enabled(enabled, egui::Button::new(t!(key)));
                crate::live::track(&response, id);
                if response.clicked() {
                    enqueue(actions, id, serde_json::Value::Null);
                }
            }
        });
    });
}
