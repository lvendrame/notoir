use super::{Association, Backend, EXTENSIONS};
#[cfg(not(windows))]
use std::process::Command;
use std::{fs, path::PathBuf};

pub struct NativeBackend {
    executable: PathBuf,
}
impl NativeBackend {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            executable: std::env::current_exe().map_err(|e| e.to_string())?,
        })
    }
    #[cfg(any(target_os = "macos", windows))]
    fn helper(
        &self,
        action: &str,
        content_type: &str,
        handler: Option<&str>,
    ) -> Result<String, String> {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let request = serde_json::json!({"action":action,"content_type":content_type,
            "handler":handler,"extensions":EXTENSIONS,"executable":self.executable,
            "store":cfg!(feature="microsoft-store")});
        fs::write(directory.path().join("request.json"), request.to_string())
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut c = Command::new(&self.executable);
            c.arg("--notoir-internal-associations");
            c
        };
        #[cfg(windows)]
        let mut command = {
            let script = directory.path().join("associations.ps1");
            fs::write(&script, include_str!("../../native/associations.ps1"))
                .map_err(|e| e.to_string())?;
            let mut c = crate::process::background_command("powershell.exe");
            c.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script);
            c
        };
        let output = command
            .arg(directory.path())
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().into());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_start_matches('\u{feff}')
            .into())
    }
}
impl Backend for NativeBackend {
    fn identity(&self) -> &str {
        if cfg!(target_os = "macos") {
            "dev.notoir.Notoir"
        } else if cfg!(windows) {
            "Notoir.File"
        } else {
            "dev.notoir.Notoir.desktop"
        }
    }
    fn platform(&self) -> &str {
        std::env::consts::OS
    }
    fn inspect(&self) -> Result<Vec<Association>, String> {
        #[cfg(any(target_os = "macos", windows))]
        {
            serde_json::from_str(&self.helper("inspect", "", None)?).map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            linux_inspect(self.identity())
        }
    }
    fn register(&self) -> Result<(), String> {
        #[cfg(any(target_os = "macos", windows))]
        {
            self.helper("register", "", None).map(|_| ())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let data = data_home()?;
            let path = data.join("applications").join(self.identity());
            let executable = std::env::var_os("APPIMAGE")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.executable.clone());
            let escaped = executable
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            let rows = self.inspect()?;
            let mimes = rows
                .iter()
                .map(|r| r.content_type.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(";");
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            if path.exists()
                && !fs::read_to_string(&path)
                    .map_err(|e| e.to_string())?
                    .contains("# dev.notoir.associations.v1")
            {
                // Packaged desktop files are already registered; don't replace them.
                return Ok(());
            }
            fs::write(&path, format!("[Desktop Entry]\n# dev.notoir.associations.v1\nType=Application\nName=Notoir\nExec=\"{escaped}\" -- %F\nTerminal=false\nMimeType={mimes};\n")).map_err(|e| e.to_string())?;
            if let Ok(output) = Command::new("update-desktop-database")
                .arg(path.parent().unwrap())
                .output()
                && !output.status.success()
            {
                return Err(String::from_utf8_lossy(&output.stderr).into());
            }
            Ok(())
        }
    }
    fn change(&self, content_type: &str, handler: Option<&str>) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.helper("change", content_type, handler).map(|_| ())
        }
        #[cfg(windows)]
        {
            // The supported Windows API is the settings handoff in confirm().
            let _ = (content_type, handler);
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or(home()?.join(".config"));
            let path = std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .map(|desktop| config.join(format!("{}-mimeapps.list", desktop.to_lowercase())))
                .find(|p| p.is_file())
                .unwrap_or_else(|| config.join("mimeapps.list"));
            fs::create_dir_all(&config).map_err(|e| e.to_string())?;
            let original = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(e.to_string()),
            };
            let target = handler.filter(|id| desktop_exists(id));
            let output = Command::new("xdg-mime")
                .args(["query", "default", content_type])
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).into());
            }
            let current = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let own = if linux_is_notoir(&current, self.identity()) {
                current.as_str()
            } else {
                self.identity()
            };
            let updated = update_defaults(&original, content_type, target, own);
            use std::io::Write;
            let mut temp = tempfile::NamedTempFile::new_in(&config).map_err(|e| e.to_string())?;
            temp.write_all(updated.as_bytes())
                .map_err(|e| e.to_string())?;
            temp.as_file().sync_all().map_err(|e| e.to_string())?;
            temp.persist(&path).map_err(|e| e.to_string())?;
            Ok(())
        }
    }
    fn needs_confirmation(&self) -> bool {
        cfg!(windows)
    }
    fn confirm(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.helper("confirm", "", None).map(|_| ())
        }
        #[cfg(not(windows))]
        {
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn home() -> Result<PathBuf, String> {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or("No home directory".into())
}
#[cfg(not(any(target_os = "macos", windows)))]
fn data_home() -> Result<PathBuf, String> {
    Ok(std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".local/share")))
}
#[cfg(not(any(target_os = "macos", windows)))]
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![data_home().unwrap_or_default()];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("XDG_DATA_DIRS").unwrap_or("/usr/local/share:/usr/share".into()),
    ));
    dirs
}
#[cfg(not(any(target_os = "macos", windows)))]
fn desktop_exists(id: &str) -> bool {
    // Desktop IDs replace subdirectory separators with dashes.
    let mut pending: Vec<_> = data_dirs()
        .into_iter()
        .map(|dir| (dir.join("applications"), String::new()))
        .collect();
    while let Some((dir, prefix)) = pending.pop() {
        if dir.join(id.strip_prefix(&prefix).unwrap_or(id)).is_file() {
            return true;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    let next = format!("{prefix}{}-", entry.file_name().to_string_lossy());
                    if id.starts_with(&next) {
                        pending.push((entry.path(), next));
                    }
                }
            }
        }
    }
    false
}
#[cfg(not(any(target_os = "macos", windows)))]
fn linux_is_notoir(id: &str, identity: &str) -> bool {
    if id == identity {
        return true;
    }
    if ![
        "notoir.desktop",
        "Notoir.desktop",
        "dev.notoir.context.desktop",
    ]
    .contains(&id)
    {
        return false;
    }
    data_dirs().iter().any(|dir| {
        fs::read_to_string(dir.join("applications").join(id)).is_ok_and(|s| {
            s.lines().any(|line| line == "Name=Notoir")
                && s.lines()
                    .any(|line| line.starts_with("Exec=") && line.to_lowercase().contains("notoir"))
        })
    })
}
#[cfg(not(any(target_os = "macos", windows)))]
fn linux_inspect(identity: &str) -> Result<Vec<Association>, String> {
    let mut globs = std::collections::HashMap::<String, (u32, String)>::new();
    for dir in data_dirs() {
        if let Ok(text) = fs::read_to_string(dir.join("mime/globs2")) {
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let mut parts = line.split(':');
                if let (Some(weight), Some(mime), Some(glob)) =
                    (parts.next(), parts.next(), parts.next())
                    && let Some(ext) = glob.strip_prefix("*.")
                {
                    let weight = weight.parse().unwrap_or(0);
                    let entry = globs.entry(ext.into()).or_insert((0, mime.into()));
                    if weight > entry.0 {
                        *entry = (weight, mime.into());
                    }
                }
            }
        }
    }
    let mut defaults = std::collections::HashMap::new();
    let mut rows = Vec::new();
    for extension in EXTENSIONS {
        let mime = globs
            .get(*extension)
            .map(|(_, m)| m.as_str())
            .unwrap_or("text/plain");
        if !defaults.contains_key(mime) {
            let output = Command::new("xdg-mime")
                .args(["query", "default", mime])
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).into());
            }
            let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            defaults.insert(
                mime.to_owned(),
                if value.is_empty() || !desktop_exists(&value) {
                    None
                } else {
                    Some(value)
                },
            );
        }
        let handler = defaults[mime].clone();
        rows.push(Association {
            extension: (*extension).into(),
            content_type: mime.into(),
            is_notoir: handler
                .as_deref()
                .is_some_and(|id| linux_is_notoir(id, identity)),
            handler,
        });
    }
    Ok(rows)
}

/// Change only the requested default entry. Comments and unrelated groups survive.
#[cfg(any(test, not(any(target_os = "macos", windows))))]
fn update_defaults(text: &str, mime: &str, handler: Option<&str>, own: &str) -> String {
    let mut out = Vec::new();
    let mut section = false;
    let mut found_section = false;
    let mut written = false;
    for line in text.lines() {
        if line.starts_with('[') {
            if section && !written {
                if let Some(h) = handler {
                    out.push(format!("{mime}={h};"));
                }
                written = true;
            }
            section = line.trim() == "[Default Applications]";
            found_section |= section;
        }
        if section
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == mime
        {
            if !written {
                let mut handlers: Vec<&str> = value
                    .split(';')
                    .filter(|h| !h.is_empty() && *h != own && Some(*h) != handler)
                    .collect();
                if let Some(h) = handler {
                    handlers.insert(0, h);
                }
                if !handlers.is_empty() {
                    out.push(format!("{mime}={};", handlers.join(";")));
                }
                written = true;
            }
        } else {
            out.push(line.into());
        }
    }
    if !written {
        if !found_section {
            out.push("[Default Applications]".into());
        }
        if let Some(h) = handler {
            out.push(format!("{mime}={h};"));
        }
    }
    format!("{}\n", out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::update_defaults;
    #[test]
    fn preserves_unrelated_mime_entries_and_restores_fallback() {
        let original = "# keep\n[Default Applications]\ntext/plain=notoir.desktop;old.desktop;\nimage/png=viewer.desktop;\n[Added Associations]\ntext/plain=notoir.desktop;\n";
        let updated = update_defaults(original, "text/plain", None, "notoir.desktop");
        assert_eq!(
            updated,
            original.replace(
                "text/plain=notoir.desktop;old.desktop;",
                "text/plain=old.desktop;"
            )
        );
        assert!(
            update_defaults(
                "[Other]\nx=y\n",
                "text/plain",
                Some("new.desktop"),
                "notoir.desktop"
            )
            .ends_with("[Default Applications]\ntext/plain=new.desktop;\n")
        );
    }
}
