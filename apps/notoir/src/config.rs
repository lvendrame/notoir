//! User settings and keybindings files (docs/specs/05, 20): opening them in
//! the editor and applying them when they are saved or changed on disk.

use std::path::{Path, PathBuf};

use notoir_core::keymap::UserBinding;
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_storage::settings_file::{self, LoadedLayer};

use crate::core::{AppCore, Effect, ThemeChoice};
use crate::i18n::Language;
use crate::io::IoResult;

#[derive(Debug)]
pub struct ConfigFiles {
    pub settings: Result<LoadedLayer, String>,
    pub bindings: Result<Vec<UserBinding>, (usize, String)>,
}

impl AppCore {
    /// Whether `path` is the user settings or keybindings file.
    pub fn is_config_file(&self, path: &Path) -> bool {
        let same = |p: PathBuf| p == path || p.canonicalize().ok().as_deref() == Some(path);
        same(self.paths.user_settings_file()) || same(self.paths.keybindings_file())
    }

    /// Opens a config file, writing `template` first when it does not exist.
    pub fn open_config_file(&mut self, path: PathBuf, template: &'static str) {
        let p = path.clone();
        self.io.submit(move || {
            if !p.exists() {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&p, template) {
                    tracing::warn!(error = %e, "could not create config file");
                }
            }
            None
        });
        self.open_path(&path, crate::files::OpenAt::default());
    }

    /// Re-reads both files off the UI thread.
    pub fn reload_config(&mut self) {
        let settings = self.paths.user_settings_file();
        let bindings = self.paths.keybindings_file();
        let previous = self.user_settings.clone();
        self.fs.submit(move || {
            let settings = settings_file::load_settings_file(&settings, Some(&previous))
                .map_err(|e| e.to_string());
            let bindings = match std::fs::read_to_string(&bindings) {
                Ok(t) => settings_file::parse_keybindings(&t).map_err(|e| (e.line, e.to_string())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(e) => Err((0, e.to_string())),
            };
            Some(IoResult::ConfigReloaded(Box::new(ConfigFiles {
                settings,
                bindings,
            })))
        });
    }

    pub fn apply_config(&mut self, files: ConfigFiles) {
        let now = self.now_ms;
        match files.settings {
            Ok(loaded) => {
                if let Some(e) = &loaded.error {
                    self.notifications.notify(
                        Notification::new(
                            Severity::Warning,
                            Translatable::new("notifications:settingsParse").arg("line", e.line),
                        )
                        .detail(e.to_string())
                        .source(NotificationSource::Settings)
                        .dedupe("settings-parse"),
                        now,
                    );
                }
                self.user_settings = loaded.layer;
                let before_theme = self.settings.str("workbench.theme", None).to_owned();
                let before_lang = self.settings.str("language", None).to_owned();
                self.apply_resolved_settings();
                let theme = self.settings.str("workbench.theme", None).to_owned();
                if theme != before_theme
                    && let Some(choice) = ThemeChoice::from_setting(&theme)
                {
                    self.theme_choice = choice;
                    self.effects.push(Effect::ApplyTheme);
                }
                let lang = self.settings.str("language", None).to_owned();
                if lang != before_lang
                    && let Some(l) = Language::from_code(&lang)
                {
                    self.language = l;
                    self.effects.push(Effect::ApplyLanguage);
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not read settings file"),
        }
        match files.bindings {
            Ok(bindings) => {
                self.keymap = crate::core::build_keymap(
                    &self.commands,
                    &bindings,
                    self.platform,
                    &mut self.notifications,
                );
                tracing::info!(count = bindings.len(), "keybindings applied");
            }
            Err((line, detail)) => {
                self.notifications.notify(
                    Notification::new(
                        Severity::Warning,
                        Translatable::new("notifications:keybindingsParse").arg("line", line),
                    )
                    .detail(detail)
                    .source(NotificationSource::Settings)
                    .dedupe("keybindings-parse"),
                    now,
                );
            }
        }
    }
}
