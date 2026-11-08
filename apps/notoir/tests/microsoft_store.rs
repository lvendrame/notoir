#![cfg(feature = "microsoft-store")]

mod common;

use notoir_platform::update::UpdateConfig;

#[test]
fn store_build_uses_package_owned_updates_and_integrations() {
    let mut app = common::start(|opts| {
        opts.update = Some(UpdateConfig {
            manifest_url: "http://127.0.0.1:1/must-not-be-requested".into(),
            public_key: "fixture".into(),
        });
    });
    let core = &mut app.harness.state_mut().core;
    assert!(
        core.updates.config.is_none(),
        "Store builds ignore updater injection"
    );
    for id in [
        "app.checkForUpdates",
        "app.downloadUpdate",
        "app.installUpdateOnQuit",
        "app.quitAndInstall",
        "app.addContextMenu",
        "app.removeContextMenu",
        "app.contextMenu",
        "app.installShellCommand",
        "app.uninstallShellCommand",
    ] {
        assert!(
            core.commands.get(&id.into()).is_none(),
            "{id} is managed by the package"
        );
    }
    assert!(core.commands.get(&"file.save".into()).is_some());
    assert!(core.commands.get(&"git.refresh".into()).is_some());
    assert!(!notoir::distribution::setting_available("update.mode"));
    assert!(notoir::distribution::setting_available("editor.fontSize"));

    core.updates.config = Some(UpdateConfig {
        manifest_url: "http://127.0.0.1:1/must-not-be-requested".into(),
        public_key: "fixture".into(),
    });
    core.updates_check(true);
    assert!(!core.updates.open);
    assert!(!core.updates.checked);
    core.updates.ready = Some(app.home.path().join("must-not-install.msi"));
    core.updates_install_on_quit();
    assert!(!core.updates.install_on_quit);

    let integration_home = app.home.path().join("integration-must-not-be-written");
    core.desktop.integration = Some(notoir_platform::file_manager::Integration {
        home: integration_home.clone(),
        target: app.home.path().join("notoir.exe"),
        label: "Notoir".into(),
        platform: notoir_platform::file_manager::Platform::Linux,
    });
    core.desktop_change(Some(true));
    assert!(!core.desktop.busy);
    core.flush_io();
    assert!(!integration_home.exists());
    assert!(
        notoir_platform::single_instance::socket_name().starts_with("dev.notoir.Notoir.store-")
    );
}
