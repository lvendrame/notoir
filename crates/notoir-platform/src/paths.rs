//! Platform-standard application directories (spec 31 §Paths).

use std::path::{Path, PathBuf};

/// Bundle identifier parts. Must never change after the first release, or user
/// data is orphaned.
pub const QUALIFIER: &str = "dev";
pub const ORGANIZATION: &str = "Notoir";
pub const APPLICATION: &str = "Notoir";
pub const BUNDLE_ID: &str = "dev.notoir.Notoir";

/// Environment variable that relocates every directory under one root
/// (portable mode, tests).
pub const HOME_ENV: &str = "NOTOIR_HOME";

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("no home directory could be determined")]
    NoHome,
    #[error("could not create {path}: {source}")]
    Create {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub recovery: PathBuf,
    pub plugins: PathBuf,
    pub scratchpads: PathBuf,
    pub themes: PathBuf,
}

impl AppPaths {
    /// Resolves directories from `NOTOIR_HOME` when set, else the OS defaults.
    pub fn resolve() -> Result<Self, PathsError> {
        match std::env::var_os(HOME_ENV) {
            Some(root) if !root.is_empty() => Ok(Self::under(Path::new(&root))),
            _ => Self::platform_default(),
        }
    }

    /// Every directory under a single root.
    pub fn under(root: &Path) -> Self {
        let data = root.join("data");
        Self::from_parts(
            root.join("config"),
            data,
            root.join("cache"),
            root.join("logs"),
        )
    }

    fn platform_default() -> Result<Self, PathsError> {
        let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or(PathsError::NoHome)?;
        let logs = if cfg!(target_os = "macos") {
            directories::BaseDirs::new()
                .ok_or(PathsError::NoHome)?
                .home_dir()
                .join("Library/Logs/Notoir")
        } else if cfg!(target_os = "windows") {
            dirs.data_local_dir().join("logs")
        } else {
            dirs.state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .join("logs")
        };
        let cache = if cfg!(target_os = "windows") {
            dirs.data_local_dir().join("cache")
        } else {
            dirs.cache_dir().to_path_buf()
        };
        // macOS (and Windows roaming) report the same directory for config and
        // data; keep user-editable config in its own subfolder there.
        let config = if dirs.config_dir() == dirs.data_dir() {
            dirs.config_dir().join("config")
        } else {
            dirs.config_dir().to_path_buf()
        };
        Ok(Self::from_parts(
            config,
            dirs.data_dir().to_path_buf(),
            cache,
            logs,
        ))
    }

    fn from_parts(config: PathBuf, data: PathBuf, cache: PathBuf, logs: PathBuf) -> Self {
        Self {
            themes: config.join("themes"),
            recovery: data.join("recovery"),
            plugins: data.join("plugins"),
            scratchpads: data.join("scratchpads"),
            config,
            data,
            cache,
            logs,
        }
    }

    /// Creates every directory (idempotent).
    pub fn ensure_dirs(&self) -> Result<(), PathsError> {
        for dir in self.all() {
            std::fs::create_dir_all(dir).map_err(|source| PathsError::Create {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        Ok(())
    }

    pub fn all(&self) -> [&Path; 8] {
        [
            &self.config,
            &self.data,
            &self.cache,
            &self.logs,
            &self.recovery,
            &self.plugins,
            &self.scratchpads,
            &self.themes,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_lives_under_a_custom_root() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(root.path());
        paths.ensure_dirs().unwrap();
        for dir in paths.all() {
            assert!(dir.starts_with(root.path()), "{}", dir.display());
            assert!(dir.is_dir(), "{}", dir.display());
        }
        assert_eq!(paths.recovery, root.path().join("data/recovery"));
    }

    #[test]
    fn platform_defaults_are_distinct_directories() {
        let paths = AppPaths::platform_default().unwrap();
        let mut all: Vec<_> = paths.all().to_vec();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 8);
    }
}
