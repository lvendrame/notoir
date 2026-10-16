//! Updates in the app (docs/specs/31 §Updates): `update.mode` = off |
//! notify | download. Checks run at most daily (and from Help → Check for
//! Updates…) on their own thread; a verified offer is announced, or
//! downloaded in `download` mode; a verified installer is handed to the
//! platform only after a normal exit, when the user chose Install on Quit.
//! Never installs silently.

use std::path::PathBuf;

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_platform::update::{self, Offer, UpdateConfig, UpdateError};

use crate::core::AppCore;
use crate::io::IoResult;

/// Automatic checks happen at most this often.
pub const CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
/// The running version, compared with the manifest's.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Default)]
pub struct UpdatesState {
    /// Manifest URL and key (built in for release builds).
    pub config: Option<UpdateConfig>,
    /// The installer kind this build updates with (`update::platform_key`).
    pub platform: Option<&'static str>,
    /// The last check time (ms since epoch) has been read from the cache.
    stamp_loaded: bool,
    last_check_ms: Option<i64>,
    checking: bool,
    downloading: bool,
    pub offer: Option<Offer>,
    /// A downloaded, verified installer.
    pub ready: Option<PathBuf>,
    pub install_on_quit: bool,
    pub last_error: Option<UpdateError>,
}

impl UpdatesState {
    pub fn new(config: Option<UpdateConfig>) -> Self {
        Self {
            config,
            platform: update::platform_key(),
            ..Default::default()
        }
    }
}

fn stamp_file(core: &AppCore) -> PathBuf {
    core.paths.cache.join("update").join("last-check")
}

fn epoch_ms() -> i64 {
    crate::mcp::epoch_ms()
}

impl AppCore {
    fn update_mode(&self) -> String {
        self.settings.str("update.mode", None).to_owned()
    }

    /// Reads the last check time (startup).
    pub fn updates_init(&mut self) {
        let file = stamp_file(self);
        self.fs.submit(move || {
            let ms = std::fs::read_to_string(file)
                .ok()
                .and_then(|s| s.trim().parse().ok());
            Some(IoResult::UpdateStamp(ms))
        });
    }

    pub fn updates_stamp(&mut self, ms: Option<i64>) {
        self.updates.stamp_loaded = true;
        self.updates.last_check_ms = ms;
    }

    /// Per frame: a daily check when enabled.
    pub fn updates_tick(&mut self) {
        let u = &self.updates;
        if !u.stamp_loaded || u.checking || u.config.is_none() || u.platform.is_none() {
            return;
        }
        if self.update_mode() == "off" {
            return;
        }
        let due = u
            .last_check_ms
            .is_none_or(|t| epoch_ms() - t >= CHECK_INTERVAL_MS);
        if due {
            self.updates_check(false);
        }
    }

    /// Checks now; `manual` (Help → Check for Updates…) reports every outcome.
    pub fn updates_check(&mut self, manual: bool) {
        let (Some(cfg), Some(platform)) = (self.updates.config.clone(), self.updates.platform)
        else {
            if manual {
                self.notify(Notification::new(
                    Severity::Info,
                    Translatable::new("notifications:updateUnavailable"),
                ));
            }
            return;
        };
        if self.updates.checking {
            return;
        }
        self.updates.checking = true;
        let now = epoch_ms();
        self.updates.last_check_ms = Some(now);
        let stamp = stamp_file(self);
        let poster = self.fs.poster();
        let spawned = std::thread::Builder::new()
            .name("notoir-update".into())
            .spawn(move || {
                if let Some(dir) = stamp.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&stamp, now.to_string());
                let result = update::check(&cfg, CURRENT, platform);
                poster.post(IoResult::UpdateChecked(Box::new((result, manual))));
            });
        if spawned.is_err() {
            self.updates.checking = false;
        }
    }

    pub fn updates_checked(&mut self, result: Result<Option<Offer>, UpdateError>, manual: bool) {
        self.updates.checking = false;
        match result {
            Ok(Some(offer)) => {
                let version = offer.version.clone();
                tracing::info!(%version, "update available");
                self.updates.offer = Some(offer);
                self.updates.last_error = None;
                if self.update_mode() == "download" {
                    self.updates_download();
                } else {
                    self.notify(
                        Notification::new(
                            Severity::Info,
                            Translatable::new("notifications:updateAvailable")
                                .arg("version", version),
                        )
                        .action(
                            "notifications:updateDownload",
                            CommandInvocation::new(
                                "app.downloadUpdate",
                                InvocationSource::Internal,
                            ),
                        ),
                    );
                }
            }
            Ok(None) => {
                self.updates.last_error = None;
                if manual {
                    self.notify(
                        Notification::new(
                            Severity::Info,
                            Translatable::new("notifications:updateUpToDate"),
                        )
                        .detail(CURRENT.to_owned()),
                    );
                }
            }
            Err(e) => {
                // A bad signature offers nothing (spec 31) and is logged.
                tracing::warn!(error = %e, "update check failed; nothing offered");
                if manual {
                    self.notify(
                        Notification::new(
                            Severity::Warning,
                            Translatable::new("notifications:updateCheckFailed"),
                        )
                        .detail(e.to_string()),
                    );
                }
                self.updates.last_error = Some(e);
            }
        }
    }

    /// Downloads the offered installer into the cache and verifies it.
    pub fn updates_download(&mut self) {
        let Some(offer) = self.updates.offer.clone() else {
            return;
        };
        if self.updates.downloading || self.updates.ready.is_some() {
            return;
        }
        self.updates.downloading = true;
        let dir = self.paths.cache.join("update");
        let poster = self.fs.poster();
        let spawned = std::thread::Builder::new()
            .name("notoir-update-download".into())
            .spawn(move || {
                let result = update::download(&offer, &dir);
                poster.post(IoResult::UpdateDownloaded(Box::new(result)));
            });
        if spawned.is_err() {
            self.updates.downloading = false;
        }
    }

    pub fn updates_downloaded(&mut self, result: Result<PathBuf, UpdateError>) {
        self.updates.downloading = false;
        let version = self
            .updates
            .offer
            .as_ref()
            .map(|o| o.version.clone())
            .unwrap_or_default();
        match result {
            Ok(path) => {
                tracing::info!(path = %path.display(), "update downloaded and verified");
                self.updates.ready = Some(path);
                self.notify(
                    Notification::new(
                        Severity::Info,
                        Translatable::new("notifications:updateReady").arg("version", version),
                    )
                    .action(
                        "notifications:updateInstallOnQuit",
                        CommandInvocation::new(
                            "app.installUpdateOnQuit",
                            InvocationSource::Internal,
                        ),
                    ),
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "update download failed");
                self.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("notifications:updateDownloadFailed"),
                    )
                    .detail(e.to_string()),
                );
                self.updates.last_error = Some(e);
            }
        }
    }

    pub fn updates_install_on_quit(&mut self) {
        if self.updates.ready.is_none() {
            return;
        }
        self.updates.install_on_quit = true;
        self.notify(Notification::new(
            Severity::Info,
            Translatable::new("notifications:updateWillInstall"),
        ));
    }

    /// At exit, after the session and recovery are written.
    pub(crate) fn updates_hand_off(&mut self) {
        if !self.updates.install_on_quit {
            return;
        }
        if let Some(path) = self.updates.ready.take() {
            tracing::info!(path = %path.display(), "handing the update to the installer");
            self.files.dialogs.install_update(&path);
        }
    }
}
