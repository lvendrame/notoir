//! The `notoir` terminal command (docs/specs/38 §Command Availability).
//! Linux packages ship `/usr/bin/notoir`; the macOS app bundle, the AppImage
//! and Windows installs get it from Help → Install 'notoir' Command:
//! a symlink in `/usr/local/bin` (macOS, with administrator authorization
//! when needed) or `~/.local/bin` (Linux), or the install folder added to
//! the user `PATH` (Windows).

use std::path::{Path, PathBuf};

/// The program the command should run: the AppImage itself when running
/// from one, else this executable.
pub fn target() -> std::io::Result<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(appimage));
    }
    std::env::current_exe()
}

/// Where the command is linked on this platform (`None` on Windows).
pub fn default_link() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from("/usr/local/bin/notoir"))
    } else if cfg!(windows) {
        None
    } else {
        directories::BaseDirs::new().map(|d| d.home_dir().join(".local/bin/notoir"))
    }
}

/// Creates or replaces the `link` → `target` symlink. Refuses to replace a
/// regular file (another program's `notoir`).
pub fn install_link(target: &Path, link: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(link) {
        Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(link)?,
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a link", link.display()),
            ));
        }
        Err(_) => {}
    }
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        Err(std::io::Error::other(
            "symlinks are not used on this platform",
        ))
    }
}

/// Removes `link` when it points to `target`; true when removed.
pub fn uninstall_link(target: &Path, link: &Path) -> std::io::Result<bool> {
    match std::fs::read_link(link) {
        Ok(to) if to == target => {
            std::fs::remove_file(link)?;
            Ok(true)
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} belongs to another program", link.display()),
        )),
        Err(_) => Ok(false),
    }
}

/// Installs the command; returns where it is.
pub fn install() -> Result<PathBuf, String> {
    let target = target().map_err(|e| e.to_string())?;
    match default_link() {
        Some(link) => match install_link(&target, &link) {
            Ok(()) => Ok(link),
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    && cfg!(target_os = "macos") =>
            {
                admin_shell(&format!(
                    "mkdir -p /usr/local/bin && ln -sfn {} {}",
                    quote(&target),
                    quote(&link)
                ))?;
                Ok(link)
            }
            Err(e) => Err(e.to_string()),
        },
        None => windows_path(&target, true),
    }
}

/// Removes the command installed by [`install`].
pub fn uninstall() -> Result<PathBuf, String> {
    let target = target().map_err(|e| e.to_string())?;
    match default_link() {
        Some(link) => match uninstall_link(&target, &link) {
            Ok(_) => Ok(link),
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    && cfg!(target_os = "macos") =>
            {
                admin_shell(&format!("rm -f {}", quote(&link)))?;
                Ok(link)
            }
            Err(e) => Err(e.to_string()),
        },
        None => windows_path(&target, false),
    }
}

fn quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Runs a shell command with administrator rights (macOS authorization
/// dialog).
fn admin_shell(command: &str) -> Result<(), String> {
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        command.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let out = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Adds (or removes) the folder of `target` to the user `PATH` (Windows).
fn windows_path(target: &Path, add: bool) -> Result<PathBuf, String> {
    let dir = target
        .parent()
        .ok_or("no install folder")?
        .to_string_lossy()
        .replace('\'', "''");
    let script = if add {
        format!(
            "$p=[Environment]::GetEnvironmentVariable('Path','User'); if(-not $p){{$p=''}}; \
             if(-not (($p -split ';') -contains '{dir}')){{[Environment]::SetEnvironmentVariable('Path',($p.TrimEnd(';')+';{dir}').TrimStart(';'),'User')}}"
        )
    } else {
        format!(
            "$p=[Environment]::GetEnvironmentVariable('Path','User'); if($p){{\
             [Environment]::SetEnvironmentVariable('Path',(($p -split ';') | Where-Object {{ $_ -ne '{dir}' -and $_ -ne '' }}) -join ';','User')}}"
        )
    };
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(PathBuf::from(dir))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn link_install_replace_and_uninstall() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("Notoir.app/Contents/MacOS/notoir");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        let link = d.path().join("bin/notoir");
        install_link(&target, &link).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        // Reinstalling (an updated app) replaces the link.
        install_link(&target, &link).unwrap();
        // Another program's link is left alone on uninstall.
        assert!(uninstall_link(Path::new("/other"), &link).is_err());
        assert!(uninstall_link(&target, &link).unwrap());
        assert!(!link.exists());
        assert!(!uninstall_link(&target, &link).unwrap());
        // A regular file is never replaced.
        std::fs::write(&link, "x").unwrap();
        assert!(install_link(&target, &link).is_err());
    }
}
