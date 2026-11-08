//! Distribution policy: Microsoft Store owns package updates, file associations
//! and execution aliases for MSIX installs.

pub const MICROSOFT_STORE: bool = cfg!(feature = "microsoft-store");

pub fn command_available(id: &str) -> bool {
    !MICROSOFT_STORE
        || !matches!(
            id,
            "app.checkForUpdates"
                | "app.downloadUpdate"
                | "app.installUpdateOnQuit"
                | "app.quitAndInstall"
                | "app.closeUpdates"
                | "app.addContextMenu"
                | "app.removeContextMenu"
                | "app.contextMenu"
                | "app.closeContextMenu"
                | "app.installShellCommand"
                | "app.uninstallShellCommand"
        )
}

pub fn setting_available(key: &str) -> bool {
    !MICROSOFT_STORE || !key.starts_with("update.")
}
