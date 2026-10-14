//! Reveal a file in the OS file manager (spec 31 §Reveal / Open).

use std::path::Path;
use std::process::Command;

pub fn reveal(path: &Path) -> std::io::Result<()> {
    if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn()?;
    } else if cfg!(target_os = "windows") {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        Command::new("explorer").arg(arg).spawn()?;
    } else {
        // FileManager1 over D-Bus selects the item where supported.
        let uri = format!("file://{}", path.display());
        let selected = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
            ])
            .arg(format!("array:string:{uri}"))
            .arg("string:")
            .status()
            .is_ok_and(|s| s.success());
        if !selected {
            let dir = path.parent().unwrap_or(path);
            Command::new("xdg-open").arg(dir).spawn()?;
        }
    }
    Ok(())
}
