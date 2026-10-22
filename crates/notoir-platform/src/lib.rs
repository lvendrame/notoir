//! Cross-platform integration (docs/specs/31-platform-packaging-and-updates.md):
//! application directories, OS locale, platform labels and system fonts.

pub mod dialogs;
pub mod download;
pub mod elevation;
pub mod paths;
pub mod reveal;
pub mod shell_command;
pub mod single_instance;
pub mod system_fonts;
pub mod trash;
pub mod update;

pub use paths::{AppPaths, PathsError};

/// OS locale as a BCP-47-ish tag (`"pt-BR"`), if the OS reports one.
pub fn os_locale() -> Option<String> {
    sys_locale::get_locale()
}

/// i18n key for the "reveal in file manager" action on this OS.
pub fn reveal_label_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "common:reveal.finder"
    } else if cfg!(target_os = "windows") {
        "common:reveal.explorer"
    } else {
        "common:reveal.fileManager"
    }
}

mod console;
pub use console::attach_parent_console;
