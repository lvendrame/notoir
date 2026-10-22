//! Help menu actions drive async per-user registration without touching the real home.
mod common;
use egui_kittest::kittest::Queryable;
use notoir_platform::file_manager::{Integration, Platform};
use serde_json::Value;
// spec 31: "File-manager integration can be added and removed"
#[test]
fn install_remove_and_preserve_foreign_registration() {
    let root = tempfile::tempdir().unwrap();
    let mut app = common::start(|_| {});
    common::settle(&mut app);
    app.harness.state_mut().core.desktop.integration = Some(Integration {
        home: root.path().into(),
        platform: Platform::Linux,
        target: std::path::PathBuf::from("/an app's path/notoir"),
        label: "Open with Notoir".into(),
    });
    app.harness.state_mut().core.desktop.installed = Some(false);
    common::run(&mut app, "app.addContextMenu", Value::Null);
    common::settle(&mut app);
    assert_eq!(app.harness.state().core.desktop.installed, Some(true));
    assert!(app.harness.query_by_label("Installed.").is_some());
    let path = root
        .path()
        .join(".local/share/nautilus/scripts/Open with Notoir");
    assert!(std::fs::read_to_string(&path).unwrap().contains("'\\''"));
    common::run(&mut app, "app.removeContextMenu", Value::Null);
    common::settle(&mut app);
    assert!(!path.exists());
    assert_eq!(app.harness.state().core.desktop.installed, Some(false));
    std::fs::write(&path, "foreign").unwrap();
    common::run(&mut app, "app.addContextMenu", Value::Null);
    common::settle(&mut app);
    assert!(app.harness.state().core.desktop.error.is_some());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "foreign");
}
// spec 31: "Manual update check reports unavailable builds"
#[test]
fn unavailable_update_has_explanation_and_dismissal() {
    let mut app = common::start(|o| o.update = None);
    app.harness.state_mut().core.updates.platform = Some("macos");
    common::run(&mut app, "app.checkForUpdates", Value::Null);
    assert!(
        app.harness
            .query_by_label("Updates are unavailable in this build.")
            .is_some()
    );
    app.harness.get_by_label("Later").click();
    app.harness.run_steps(3);
    assert!(!app.harness.state().core.updates.open);
}

#[test]
fn package_managed_install_has_package_manager_guidance() {
    let mut app = common::start(|_| {});
    app.harness.state_mut().core.updates.platform = None;
    common::run(&mut app, "app.checkForUpdates", Value::Null);
    assert!(
        app.harness
            .query_by_label("Update Notoir using your package manager.")
            .is_some()
    );
}
