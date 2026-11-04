//! Process-wide serialization for shared read/modify/write operations.
//! Acquire only from workers, never while running UI code.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub fn for_path(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let key = path.canonicalize().unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| parent.canonicalize().ok())
            .zip(path.file_name())
            .map(|(parent, name)| parent.join(name))
            .unwrap_or_else(|| path.to_path_buf())
    });
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

#[cfg(test)]
mod tests {
    #[test]
    fn concurrent_settings_updates_preserve_every_key() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("settings.toml");
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    crate::settings_file::write_setting_file(
                        &path,
                        &format!("plugin.setting{i}"),
                        None,
                        Some(&notoir_core::settings::SettingValue::Int(i)),
                    )
                    .unwrap()
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let loaded = crate::settings_file::load_settings_file(&path, None).unwrap();
        assert_eq!(loaded.layer.values.len(), 16);
    }
}
