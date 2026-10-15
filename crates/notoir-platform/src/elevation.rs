//! Writing a protected file with administrator authorization
//! (docs/specs/36-protected-file-saving.md). The bytes go to a temp file as
//! the user; one elevated command then replaces the target, keeping its
//! owner and permissions (copy the original to a sibling, overwrite that
//! copy, rename it over the target). Authorization covers this one command
//! only.

use std::path::Path;
use std::process::Command;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ElevationError {
    #[error("authorization cancelled")]
    Cancelled,
    #[error("administrator save is not supported on this platform")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
}

/// Writes files that need administrator rights. Tests script it.
pub trait Elevation: Send + Sync {
    /// Replaces (or creates) `path` with `bytes` after asking for
    /// authorization. Blocks until the user answered; call off the UI thread.
    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<(), ElevationError>;
}

/// The platform's own elevation flow: `osascript … with administrator
/// privileges` (macOS), `pkexec` (Linux), UAC through PowerShell (Windows).
pub struct NativeElevation;

/// POSIX single-quoted shell word.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The shell command that installs `src` as `dst`.
fn posix_script(src: &Path, dst: &Path) -> String {
    let (s, d) = (
        sh_quote(&src.to_string_lossy()),
        sh_quote(&dst.to_string_lossy()),
    );
    let tmp = sh_quote(&format!("{}.notoir-save", dst.to_string_lossy()));
    format!(
        "if [ -e {d} ]; then /bin/cp -p {d} {tmp} && /bin/cat {s} > {tmp} && /bin/mv -f {tmp} {d}; \
         else /bin/cp {s} {d}; fi"
    )
}

fn temp_copy(bytes: &[u8]) -> Result<tempfile_path::TempPath, ElevationError> {
    tempfile_path::write(bytes).map_err(|e| ElevationError::Failed(e.to_string()))
}

/// A user-owned temp file removed on drop.
mod tempfile_path {
    use std::path::{Path, PathBuf};

    pub struct TempPath(PathBuf);

    impl TempPath {
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    pub fn write(bytes: &[u8]) -> std::io::Result<TempPath> {
        let name = format!(
            "notoir-admin-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        let p = std::env::temp_dir().join(name);
        std::fs::write(&p, bytes)?;
        Ok(TempPath(p))
    }
}

fn run(
    mut cmd: Command,
    cancelled: impl Fn(Option<i32>, &str) -> bool,
) -> Result<(), ElevationError> {
    let out = cmd
        .output()
        .map_err(|e| ElevationError::Failed(e.to_string()))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if cancelled(out.status.code(), &err) {
        Err(ElevationError::Cancelled)
    } else {
        Err(ElevationError::Failed(if err.is_empty() {
            format!("exit status {:?}", out.status.code())
        } else {
            err
        }))
    }
}

impl Elevation for NativeElevation {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<(), ElevationError> {
        let tmp = temp_copy(bytes)?;
        if cfg!(target_os = "macos") {
            let script = posix_script(tmp.path(), path)
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            let mut cmd = Command::new("/usr/bin/osascript");
            cmd.arg("-e").arg(format!(
                "do shell script \"{script}\" with administrator privileges"
            ));
            // -128: "User canceled."
            run(cmd, |_, err| err.contains("-128"))
        } else if cfg!(target_os = "windows") {
            let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
            let inner = format!(
                "Copy-Item -LiteralPath '{}' -Destination '{}' -Force",
                q(tmp.path()),
                q(path)
            );
            let mut cmd = Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
                     -ArgumentList '-NoProfile','-Command',\"{}\"; exit $p.ExitCode",
                    inner.replace('"', "`\"")
                ),
            ]);
            run(cmd, |_, err| err.contains("canceled by the user"))
        } else if cfg!(unix) {
            let mut cmd = Command::new("pkexec");
            cmd.args(["/bin/sh", "-c", &posix_script(tmp.path(), path)]);
            // 126: the dialog was dismissed; 127: not authorized.
            run(cmd, |code, _| matches!(code, Some(126) | Some(127)))
        } else {
            Err(ElevationError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_script() {
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
        let s = posix_script(Path::new("/tmp/x"), Path::new("/etc/my file"));
        assert!(s.contains("'/etc/my file.notoir-save'"));
        assert!(s.contains("/bin/cp -p '/etc/my file'"));
    }

    #[cfg(unix)]
    #[test]
    fn posix_script_replaces_content_and_keeps_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("t.conf");
        std::fs::write(&dst, "old").unwrap();
        std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o640)).unwrap();
        let src = dir.path().join("src");
        std::fs::write(&src, "new").unwrap();
        let ok = Command::new("/bin/sh")
            .args(["-c", &posix_script(&src, &dst)])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "new");
        let mode = std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert!(!dir.path().join("t.conf.notoir-save").exists());
    }
}
