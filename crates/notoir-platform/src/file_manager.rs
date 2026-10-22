//! Opt-in, per-user file-manager integration. Never changes default handlers.
use std::{fs, path::PathBuf, process::Command};
const OWNER: &str = "dev.notoir.context.v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}
#[derive(Clone, Debug)]
pub struct Integration {
    pub platform: Platform,
    pub home: PathBuf,
    pub target: PathBuf,
    pub label: String,
}
impl Integration {
    pub fn native(label: String) -> Result<Self, String> {
        let home = directories::BaseDirs::new()
            .ok_or("No home directory")?
            .home_dir()
            .to_path_buf();
        Ok(Self {
            platform: if cfg!(target_os = "macos") {
                Platform::Mac
            } else if cfg!(windows) {
                Platform::Windows
            } else {
                Platform::Linux
            },
            home,
            target: crate::shell_command::target().map_err(|e| e.to_string())?,
            label,
        })
    }
    fn files(&self) -> Vec<(PathBuf, String)> {
        let target = quote(&self.target.to_string_lossy());
        match self.platform {
            Platform::Mac => {
                let root = self
                    .home
                    .join("Library/Services/Open with Notoir.workflow/Contents");
                let info = format!(
                    r#"<?xml version="1.0"?><plist version="1.0"><dict><key>NotoirOwner</key><string>{OWNER}</string><key>CFBundleIdentifier</key><string>dev.notoir.open-service</string><key>NSServices</key><array><dict><key>NSMenuItem</key><dict><key>default</key><string>{}</string></dict><key>NSMessage</key><string>runWorkflowAsService</string><key>NSRequiredContext</key><dict><key>NSApplicationIdentifier</key><string>com.apple.finder</string></dict><key>NSSendFileTypes</key><array><string>public.item</string></array></dict></array></dict></plist>"#,
                    xml(&self.label)
                );
                let script = format!("# {OWNER}\nexec {target} -- \"$@\" >/dev/null 2>&1 &");
                let workflow = format!(
                    r#"<?xml version="1.0"?><plist version="1.0"><dict><key>NotoirOwner</key><string>{OWNER}</string><key>AMApplicationBuild</key><string>523</string><key>AMApplicationVersion</key><string>2.10</string><key>AMDocumentVersion</key><string>2</string><key>actions</key><array><dict><key>action</key><dict><key>ActionBundlePath</key><string>/System/Library/Automator/Run Shell Script.action</string><key>ActionName</key><string>Run Shell Script</string><key>ActionParameters</key><dict><key>COMMAND_STRING</key><string>{}</string><key>CheckedForUserDefaultShell</key><true/><key>inputMethod</key><integer>1</integer><key>shell</key><string>/bin/sh</string></dict><key>BundleIdentifier</key><string>com.apple.RunShellScript</string><key>CFBundleVersion</key><string>2.0.3</string><key>AMAccepts</key><dict><key>Container</key><string>List</string><key>Optional</key><true/><key>Types</key><array><string>com.apple.cocoa.path</string></array></dict><key>AMProvides</key><dict><key>Container</key><string>List</string><key>Types</key><array><string>com.apple.cocoa.string</string></array></dict><key>UUID</key><string>E570955A-A0C9-4FE8-978B-6AD6B4B8FCCC</string></dict></dict></array><key>workflowMetaData</key><dict><key>serviceInputTypeIdentifier</key><string>com.apple.Automator.fileSystemObject</string><key>serviceOutputTypeIdentifier</key><string>com.apple.Automator.nothing</string><key>serviceProcessesInput</key><integer>0</integer><key>workflowTypeIdentifier</key><string>com.apple.Automator.servicesMenu</string><key>serviceApplicationBundleID</key><string>com.apple.finder</string></dict></dict></plist>"#,
                    xml(&script)
                );
                vec![
                    (root.join("Info.plist"), info),
                    (root.join("document.wflow"), workflow),
                ]
            }
            Platform::Linux => {
                let data = self.home.join(".local/share");
                let desktop_target = desktop_quote(&self.target.to_string_lossy());
                vec![
                    (
                        data.join("nautilus/scripts/Open with Notoir"),
                        format!("#!/bin/sh\n# {OWNER}\nexec {target} -- \"$@\"\n"),
                    ),
                    (
                        data.join("kio/servicemenus/notoir.desktop"),
                        format!(
                            "[Desktop Entry]\n# {OWNER}\nType=Service\nMimeType=all/all;\nActions=OpenInNotoir;\n[Desktop Action OpenInNotoir]\nName={}\nIcon=accessories-text-editor\nExec={desktop_target} -- %F\n",
                            self.label
                        ),
                    ),
                    (
                        data.join("applications/dev.notoir.context.desktop"),
                        format!(
                            "[Desktop Entry]\n# {OWNER}\nType=Application\nName=Notoir\nNoDisplay=true\nExec={desktop_target} -- %F\nMimeType=text/plain;text/markdown;application/json;application/toml;application/yaml;text/csv;\n"
                        ),
                    ),
                ]
            }
            Platform::Windows => vec![],
        }
    }
    pub fn installed(&self) -> Result<bool, String> {
        if self.platform == Platform::Windows {
            return self.windows(None);
        }
        let mut all = true;
        for (path, _) in self.files() {
            if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("{} is a symbolic link", path.display()));
            }
            match fs::read_to_string(&path) {
                Ok(s) if s.contains(OWNER) => {}
                Ok(_) => return Err(format!("{} belongs to another program", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => all = false,
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(all)
    }
    pub fn change(&self, install: bool) -> Result<(), String> {
        if self.platform == Platform::Windows {
            return self.windows(Some(install)).map(|_| ());
        }
        // Preflight every path before changing anything.
        self.installed()?;
        for (path, text) in self.files() {
            if install {
                fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
                fs::write(&path, text).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                if self.platform == Platform::Linux {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                        .map_err(|e| e.to_string())?;
                }
            } else if path.exists() {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        if self.platform == Platform::Mac
            && cfg!(target_os = "macos")
            && directories::BaseDirs::new().is_some_and(|d| d.home_dir() == self.home)
        {
            // Rebuild Services cache; no GUI automation or administrator access.
            let status = Command::new("/System/Library/CoreServices/pbs")
                .arg("-update")
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("Services cache refresh failed".into());
            }
        }
        if self.platform == Platform::Linux && cfg!(target_os = "linux") {
            let _ = Command::new("update-desktop-database")
                .arg(self.home.join(".local/share/applications"))
                .status();
        }
        Ok(())
    }
    fn windows(&self, change: Option<bool>) -> Result<bool, String> {
        let roots = [
            r"HKCU\Software\Classes\*\shell\Notoir",
            r"HKCU\Software\Classes\Directory\shell\Notoir",
            r"HKCU\Software\Classes\Directory\Background\shell\Notoir",
        ];
        let mut installed = true;
        for root in roots {
            let existing = Command::new("reg.exe")
                .args(["query", root, "/v", "NotoirOwner"])
                .output()
                .map_err(|e| e.to_string())?;
            if existing.status.success() {
                if !String::from_utf8_lossy(&existing.stdout).contains(OWNER) {
                    return Err("Foreign context-menu registration".into());
                }
            } else {
                let exists = Command::new("reg.exe")
                    .args(["query", root])
                    .output()
                    .map_err(|e| e.to_string())?;
                if exists.status.success() {
                    return Err("Foreign context-menu registration".into());
                }
                installed = false;
            }
        }
        if let Some(add) = change {
            for root in roots {
                let run = |args: &[&str]| -> Result<(), String> {
                    let o = Command::new("reg.exe")
                        .args(args)
                        .output()
                        .map_err(|e| e.to_string())?;
                    if o.status.success() {
                        Ok(())
                    } else {
                        Err(String::from_utf8_lossy(&o.stderr).into_owned())
                    }
                };
                if add {
                    run(&["add", root, "/v", "NotoirOwner", "/d", OWNER, "/f"])?;
                    run(&["add", root, "/ve", "/d", &self.label, "/f"])?;
                    run(&[
                        "add",
                        root,
                        "/v",
                        "Icon",
                        "/d",
                        &self.target.display().to_string(),
                        "/f",
                    ])?;
                    let token = if root.contains("Background") {
                        "%V"
                    } else {
                        "%1"
                    };
                    // End directory arguments with a dot so a drive root's
                    // trailing backslash cannot escape the closing quote.
                    let argument = if root.contains("Directory") {
                        format!("{token}\\.")
                    } else {
                        token.to_owned()
                    };
                    let command = format!("\"{}\" -- \"{argument}\"", self.target.display());
                    run(&[
                        "add",
                        &format!(r"{root}\command"),
                        "/ve",
                        "/d",
                        &command,
                        "/f",
                    ])?;
                } else {
                    let exists = Command::new("reg.exe")
                        .args(["query", root])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if exists.status.success() {
                        run(&["delete", root, "/f"])?;
                    }
                }
            }
        }
        Ok(installed)
    }
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn desktop_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\\\\\")
            .replace('"', "\\\\\\\"")
            .replace('`', "\\\\`")
            .replace('$', "\\\\$")
            .replace('%', "%%")
    )
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_is_repeatable_and_preserves_foreign_files() {
        for platform in [Platform::Mac, Platform::Linux] {
            let dir = tempfile::tempdir().unwrap();
            let i = Integration {
                platform,
                home: dir.path().into(),
                target: std::path::Path::new("/some app's/Notoir & test").into(),
                label: "Open with Notoir".into(),
            };
            assert!(!i.installed().unwrap());
            i.change(true).unwrap();
            i.change(true).unwrap();
            assert!(i.installed().unwrap());
            for (p, s) in i.files() {
                assert_eq!(fs::read_to_string(p).unwrap(), s);
            }
            i.change(false).unwrap();
            i.change(false).unwrap();
            assert!(!i.installed().unwrap());
            let path = i.files()[0].0.clone();
            fs::write(&path, "foreign").unwrap();
            assert!(i.change(true).is_err());
            assert!(i.change(false).is_err());
            assert_eq!(fs::read_to_string(path).unwrap(), "foreign");
        }
    }
}
