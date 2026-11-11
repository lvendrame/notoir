//! Host-level tests use real editor cores, IO, sessions and recovery, without OS windows.
use notoir::application::{ApplicationHost, ApplicationRequest, MANIFEST_KEY};
use notoir::core::StartOptions;
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::settings::SettingValue;
use std::time::{Duration, Instant};

fn make_host(home: &std::path::Path, restore: bool) -> (egui::Context, ApplicationHost) {
    let ctx = egui::Context::default();
    let mut opts = StartOptions::new(notoir_platform::AppPaths::under(home));
    opts.language_override = Some(notoir::i18n::Language::En);
    opts.dialogs = Some(std::sync::Arc::new(
        notoir::files::ScriptedDialogs::default(),
    ));
    opts.update = None;
    let host = ApplicationHost::with_options(&ctx, opts, restore);
    (ctx, host)
}

fn command(host: &mut ApplicationHost, index: usize, id: &'static str) {
    let core = &mut host.windows[index].app.core;
    core.run_now(
        CommandInvocation::new(id, InvocationSource::Internal),
        &core.context(false),
    );
}

fn settle(host: &mut ApplicationHost) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        for w in &mut host.windows {
            w.app.core.flush_io();
            w.app.core.pump_io();
        }
        if host
            .windows
            .iter()
            .all(|w| w.app.core.session.restore.is_none())
        {
            break;
        }
        assert!(Instant::now() < deadline, "restore did not finish");
    }
}

#[test]
fn new_window_is_a_host_effect_and_buffers_are_independent() {
    let home = tempfile::tempdir().unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    let first = host.windows[0].id.clone();
    command(&mut host, 0, "file.newWindow");
    let core = &host.windows[0].app.core;
    assert!(matches!(
        core.effects.last(),
        Some(notoir::core::Effect::Application(
            ApplicationRequest::NewWindow(None)
        ))
    ));
    host.request(&ctx, &first, ApplicationRequest::NewWindow(None));
    command(&mut host, 0, "file.newUntitled");
    assert_eq!(host.windows.len(), 2);
    assert_ne!(host.windows[0].id, host.windows[1].id);
    assert_ne!(host.windows[0].viewport(), host.windows[1].viewport());
    assert!(!host.windows[0].app.core.workbench.documents.is_empty());
    assert!(host.windows[1].app.core.workbench.documents.is_empty());
    assert_ne!(
        host.windows[0].app.core.session_key(),
        host.windows[1].app.core.session_key()
    );
}

// spec 40: "Closing the first editor preserves its siblings"
#[test]
fn closing_first_editor_keeps_other_workbenches() {
    let home = tempfile::tempdir().unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    let first = host.windows[0].id.clone();
    host.create_window(&ctx, None, None);
    let last = host.create_window(&ctx, None, None);
    host.request(&ctx, &first, ApplicationRequest::Close);
    host.lifecycle(&ctx);
    assert_eq!(host.windows.len(), 2);
    assert!(host.windows.iter().all(|w| w.id != first));
    command(&mut host, 1, "file.newUntitled");
    assert!(!host.windows[1].app.core.workbench.documents.is_empty());
    host.request(&ctx, &last, ApplicationRequest::NewWindow(None));
    assert_eq!(host.windows.len(), 3);
}

// spec 40: "An existing workspace is activated"
#[test]
fn workspace_identity_deduplicates_new_window_requests() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    let id = host.create_window(&ctx, None, Some(&root));
    let count = host.windows.len();
    let duplicate = host.create_window(&ctx, None, Some(&root.join(".")));
    assert_eq!(duplicate, id);
    assert_eq!(host.active.as_deref(), Some(id.as_str()));
    assert_eq!(host.windows.len(), count);
}

// spec 40: "Workspace-free windows have independent sessions"
#[test]
fn restore_manifest_keeps_distinct_workspace_free_tabs_and_identity() {
    let home = tempfile::tempdir().unwrap();
    let a = home.path().join("a.txt");
    let b = home.path().join("b.txt");
    std::fs::write(&a, "first").unwrap();
    std::fs::write(&b, "second").unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    host.windows[0].app.core.open_path(&a, Default::default());
    let second = host.create_window(&ctx, None, None);
    host.windows[1].app.core.open_path(&b, Default::default());
    settle(&mut host);
    let ids: Vec<_> = host.windows.iter().map(|w| w.id.clone()).collect();
    host.persist(true);
    assert!(host.flush_storage());
    for w in &mut host.windows {
        w.app.core.shutdown();
    }
    drop(host);
    let (_, mut restored) = make_host(home.path(), true);
    settle(&mut restored);
    assert_eq!(
        restored
            .windows
            .iter()
            .map(|w| w.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(restored.active, Some(second));
    for (i, text) in ["first", "second"].iter().enumerate() {
        let wb = &restored.windows[i].app.core.workbench;
        assert_eq!(wb.documents.len(), 1);
        assert_eq!(
            wb.document(wb.active_document().unwrap())
                .unwrap()
                .buffer
                .text(),
            *text
        );
    }
}

// spec 40: "Cancelling Quit keeps all windows open"
#[test]
fn cancelled_quit_does_not_destroy_previously_ready_windows() {
    let home = tempfile::tempdir().unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    let first = host.windows[0].id.clone();
    host.create_window(&ctx, None, None);
    host.windows[1]
        .app
        .core
        .set_user_setting("app.hotExit", Some(SettingValue::Bool(false)));
    command(&mut host, 1, "file.newUntitled");
    // Untitled empty buffers are not dirty: insert an edit via the same workbench transaction API.
    let wb = &mut host.windows[1].app.core.workbench;
    let doc = wb.active_document().unwrap();
    wb.document_mut(doc)
        .unwrap()
        .buffer
        .edit(notoir_editor::EditRequest {
            changes: notoir_editor::ChangeSet::insert(0, "unsaved"),
            selection_before: Default::default(),
            selection_after: Default::default(),
            kind: notoir_editor::EditKind::Other,
            now_ms: 0,
        })
        .unwrap();
    host.request(&ctx, &first, ApplicationRequest::Quit);
    host.lifecycle(&ctx);
    assert_eq!(host.windows.len(), 2);
    assert!(!host.windows[1].app.core.files.prompts.is_empty());
    host.windows[1]
        .app
        .core
        .answer_prompt(notoir::files::Answer::Cancel);
    host.lifecycle(&ctx);
    assert_eq!(host.windows.len(), 2);
    assert!(!host.windows.iter().any(|w| w.app.core.close_ready));
    host.request(&ctx, &first, ApplicationRequest::NewWindow(None));
    assert_eq!(host.windows.len(), 3);
}

#[test]
fn explicit_close_removes_only_that_window_from_manifest() {
    let home = tempfile::tempdir().unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    let first = host.windows[0].id.clone();
    let second = host.create_window(&ctx, None, None);
    host.request(&ctx, &first, ApplicationRequest::Close);
    host.lifecycle(&ctx);
    assert!(host.flush_storage());
    let (db, _) =
        notoir_storage::StateDb::open(&host.windows[0].app.core.paths.data.join("state.db"))
            .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&db.load_session(MANIFEST_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(manifest["windows"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["windows"][0]["id"], second);
}

#[test]
fn theme_drafts_are_local_and_saved_presets_refresh_siblings() {
    let home = tempfile::tempdir().unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    host.create_window(&ctx, None, None);
    for _ in 0..3 {
        settle(&mut host);
    }
    for index in 0..2 {
        command(&mut host, index, "preferences.manageThemes");
    }
    host.windows[0].app.core.theme_action(
        &serde_json::json!({"action":"edit","path":["ui","accent"],"value":"#123456"}),
    );
    assert_ne!(
        host.windows[0]
            .app
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .theme()
            .unwrap()
            .color(notoir::theme::Tok::Accent),
        host.windows[1]
            .app
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .theme()
            .unwrap()
            .color(notoir::theme::Tok::Accent)
    );
    host.windows[0]
        .app
        .core
        .theme_action(&serde_json::json!({"action":"save"}));
    for _ in 0..3 {
        settle(&mut host);
    }
    assert!(
        host.windows[0]
            .app
            .core
            .effects
            .iter()
            .any(|effect| matches!(
                effect,
                notoir::core::Effect::Application(ApplicationRequest::RefreshThemes)
            ))
    );
    let source = host.windows[0].id.clone();
    host.request(&ctx, &source, ApplicationRequest::RefreshThemes);
    for _ in 0..3 {
        settle(&mut host);
    }
    assert_eq!(
        host.windows[1]
            .app
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .theme()
            .unwrap()
            .color(notoir::theme::Tok::Accent),
        egui::Color32::from_rgb(18, 52, 86)
    );
    assert!(
        !host.windows[1]
            .app
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .dirty()
    );
}

#[test]
fn cancelling_a_dirty_theme_close_cancels_window_shutdown() {
    let home = tempfile::tempdir().unwrap();
    let (ctx, mut host) = make_host(home.path(), false);
    command(&mut host, 0, "preferences.manageThemes");
    host.windows[0].app.core.theme_action(
        &serde_json::json!({"action":"edit","path":["ui","accent"],"value":"#123456"}),
    );
    let id = host.windows[0].id.clone();
    host.request(&ctx, &id, ApplicationRequest::Close);
    host.lifecycle(&ctx);
    assert!(host.windows[0].app.core.theme_manager.pending.is_some());
    host.windows[0]
        .app
        .core
        .theme_action(&serde_json::json!({"action":"cancelPending"}));
    host.lifecycle(&ctx);
    assert_eq!(host.windows.len(), 1);
    assert!(!host.windows[0].app.core.is_closing);
    assert!(host.windows[0].app.core.theme_manager.pending.is_none());
    assert!(
        host.windows[0]
            .app
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .dirty()
    );
}
