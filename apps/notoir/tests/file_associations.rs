mod common;
use egui_kittest::kittest::Queryable;
use notoir_core::view::ViewKind;
use notoir_platform::associations::{Association, Backend};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    selected: Mutex<bool>,
    markdown: Mutex<Option<bool>>,
    no_fallback: Mutex<bool>,
    fail: Mutex<bool>,
    pending: Mutex<bool>,
    changes: Mutex<usize>,
}
impl Backend for Fake {
    fn identity(&self) -> &str {
        "Notoir"
    }
    fn platform(&self) -> &str {
        "test"
    }
    fn inspect(&self) -> Result<Vec<Association>, String> {
        let own = *self.selected.lock().unwrap();
        let mut rows: Vec<_> = ["txt", "log"]
            .iter()
            .map(|ext| Association {
                extension: (*ext).into(),
                content_type: "text/plain".into(),
                handler: if !own && *self.no_fallback.lock().unwrap() {
                    None
                } else {
                    Some(if own { "Notoir" } else { "Previous" }.into())
                },
                is_notoir: own,
            })
            .collect();
        if let Some(own) = *self.markdown.lock().unwrap() {
            rows.push(Association {
                extension: "md".into(),
                content_type: "text/markdown".into(),
                handler: Some(if own { "Notoir" } else { "External" }.into()),
                is_notoir: own,
            });
        }
        Ok(rows)
    }
    fn register(&self) -> Result<(), String> {
        Ok(())
    }
    fn change(&self, content_type: &str, handler: Option<&str>) -> Result<(), String> {
        *self.changes.lock().unwrap() += 1;
        if *self.fail.lock().unwrap() {
            return Err("Scripted failure".into());
        }
        if !*self.pending.lock().unwrap() {
            if content_type == "text/markdown" {
                *self.markdown.lock().unwrap() = Some(handler == Some("Notoir"));
            } else {
                *self.selected.lock().unwrap() = handler == Some("Notoir");
            }
        }
        Ok(())
    }
    fn needs_confirmation(&self) -> bool {
        *self.pending.lock().unwrap()
    }
}
fn start(backend: &Arc<Fake>) -> common::TestApp {
    let backend = backend.clone();
    common::start(move |o| o.association_backend = Some(backend))
}
fn open(app: &mut common::TestApp) {
    common::run(app, "app.fileAssociations", json!(null));
    common::settle(app);
}
fn select(app: &mut common::TestApp, selected: bool) {
    common::run(
        app,
        "app.selectAssociation",
        json!({"type":"text/plain","selected":selected}),
    );
}

// spec 31: "Manage default file associations in a tab"
#[test]
fn menu_opens_single_tab_with_os_checked_extensions() {
    let backend = Arc::new(Fake::default());
    *backend.selected.lock().unwrap() = true;
    let mut app = start(&backend);
    app.harness.get_by_label("Help").click();
    app.harness.run_steps(2);
    app.harness
        .get_by_label_contains("File Associations…")
        .click();
    common::settle(&mut app);
    let view = app.harness.state().core.workbench.active_view().unwrap();
    assert_eq!(
        app.harness.state().core.workbench.view(view).unwrap().kind,
        ViewKind::FileAssociations
    );
    assert!(
        app.harness
            .state()
            .core
            .associations
            .rows
            .iter()
            .all(|r| r.is_notoir)
    );
    assert!(app.harness.state().core.associations.desired["text/plain"]);
    assert!(app.harness.query_by_label(".txt").is_some());
    assert!(app.harness.query_by_label(".log").is_some());
    open(&mut app);
    assert_eq!(app.harness.state().core.workbench.active_view(), Some(view));
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .views
            .iter()
            .filter(|v| v.kind == ViewKind::FileAssociations)
            .count(),
        1
    );
}

// spec 31: "Save associations and restore the previous default"
#[test]
fn linked_checkboxes_save_then_restore_and_close() {
    let backend = Arc::new(Fake::default());
    let mut app = start(&backend);
    open(&mut app);
    app.harness.get_by_label(".txt").click();
    app.harness.run_steps(3);
    assert!(app.harness.state().core.associations.desired["text/plain"]);
    assert!(
        app.harness
            .state()
            .core
            .associations
            .rows
            .iter()
            .filter(|r| r.content_type == "text/plain")
            .all(|r| app.harness.state().core.associations.desired[&r.content_type])
    );
    app.harness.get_by_label("Save").click();
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_none());
    assert!(*backend.selected.lock().unwrap());
    assert_eq!(*backend.changes.lock().unwrap(), 1);
    open(&mut app);
    select(&mut app, false);
    common::run(&mut app, "app.saveAssociations", json!(null));
    common::settle(&mut app);
    assert!(!*backend.selected.lock().unwrap());
    assert!(app.harness.state().core.associations.view.is_none());
    assert_eq!(*backend.changes.lock().unwrap(), 2);
}

// spec 31: "Cancel unsaved file associations"
#[test]
fn cancel_does_not_touch_os_defaults_and_stale_results_do_not_reopen_tab() {
    let backend = Arc::new(Fake::default());
    let mut app = start(&backend);
    open(&mut app);
    select(&mut app, true);
    let generation = app.harness.state().core.associations.generation;
    app.harness.get_by_label("Cancel").click();
    common::settle(&mut app);
    assert!(!*backend.selected.lock().unwrap());
    assert_eq!(*backend.changes.lock().unwrap(), 0);
    app.harness
        .state_mut()
        .core
        .associations_done(notoir::associations::Done {
            generation,
            saving: false,
            result: Ok(Default::default()),
        });
    assert!(app.harness.state().core.associations.view.is_none());
    open(&mut app);
    assert!(!app.harness.state().core.associations.desired["text/plain"]);
}

// spec 31: "Keep association changes open until verified"
#[test]
fn failure_and_pending_confirmation_keep_tab_open_until_retry_or_refresh() {
    let backend = Arc::new(Fake::default());
    *backend.fail.lock().unwrap() = true;
    let mut app = start(&backend);
    open(&mut app);
    select(&mut app, true);
    common::run(&mut app, "app.saveAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_some());
    assert_eq!(
        app.harness.state().core.associations.errors["text/plain"],
        "Scripted failure"
    );
    *backend.fail.lock().unwrap() = false;
    *backend.pending.lock().unwrap() = true;
    common::run(&mut app, "app.saveAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.pending);
    assert!(app.harness.state().core.associations.view.is_some());
    *backend.selected.lock().unwrap() = true;
    common::run(&mut app, "app.refreshAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_none());
}

#[test]
fn tab_close_discards_draft_and_new_request_ignores_old_generation() {
    let backend = Arc::new(Fake::default());
    let mut app = start(&backend);
    open(&mut app);
    select(&mut app, true);
    let generation = app.harness.state().core.associations.generation;
    let view = app.harness.state().core.associations.view.unwrap();
    app.harness.state_mut().core.close_view_now(view);
    open(&mut app);
    assert!(!app.harness.state().core.associations.desired["text/plain"]);
    app.harness
        .state_mut()
        .core
        .associations_done(notoir::associations::Done {
            generation,
            saving: true,
            result: Err("stale".into()),
        });
    assert!(app.harness.state().core.associations.error.is_none());
}

#[test]
fn association_tab_snapshots() {
    for theme in [
        notoir::theme::ThemeId::Midnight,
        notoir::theme::ThemeId::Paper,
        notoir::theme::ThemeId::Dracula,
    ] {
        let backend = Arc::new(Fake::default());
        let mut app = start(&backend);
        app.harness.state_mut().core.associations.backend = Some(backend.clone());
        common::run(
            &mut app,
            "preferences.changeTheme",
            json!({"theme":theme.id()}),
        );
        open(&mut app);
        // Exercise the full catalog without touching the OS.
        app.harness.state_mut().core.associations.rows = notoir_platform::associations::EXTENSIONS
            .iter()
            .map(|ext| Association {
                extension: (*ext).into(),
                content_type: (*ext).into(),
                handler: None,
                is_notoir: false,
            })
            .collect();
        app.harness
            .state_mut()
            .core
            .associations
            .rows
            .sort_by(|a, b| a.extension.cmp(&b.extension));
        app.harness.run_steps(3);
        let out =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots");
        std::fs::create_dir_all(&out).unwrap();
        for (name, width) in [("desktop", 1280.0), ("narrow", 620.0)] {
            app.harness.set_size(egui::vec2(width, 800.0));
            app.harness.run_steps(3);
            if let Ok(image) = app.harness.render() {
                image
                    .save(out.join(format!("file-associations-{}-{name}.png", theme.id())))
                    .unwrap();
            }
        }
    }
}

#[test]
fn save_preserves_external_changes_to_unedited_types() {
    let backend = Arc::new(Fake::default());
    *backend.markdown.lock().unwrap() = Some(true);
    let mut app = start(&backend);
    open(&mut app);
    select(&mut app, true);
    *backend.markdown.lock().unwrap() = Some(false);
    common::run(&mut app, "app.saveAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_none());
    assert!(!app.harness.state().core.associations.desired["text/markdown"]);
    assert_eq!(*backend.changes.lock().unwrap(), 1);
    assert_eq!(*backend.markdown.lock().unwrap(), Some(false));
}

#[test]
fn missing_replacement_keeps_tab_open_after_refresh() {
    let backend = Arc::new(Fake::default());
    *backend.selected.lock().unwrap() = true;
    let mut app = start(&backend);
    open(&mut app);
    select(&mut app, false);
    *backend.no_fallback.lock().unwrap() = true;
    common::run(&mut app, "app.saveAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_some());
    common::run(&mut app, "app.refreshAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_some());
    assert!(app.harness.state().core.associations.pending);
    common::run(&mut app, "app.saveAssociations", json!(null));
    common::settle(&mut app);
    assert!(app.harness.state().core.associations.view.is_some());
}
