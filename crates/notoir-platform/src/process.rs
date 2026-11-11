//! Background console helpers must not create terminal windows on Windows.
//! Redirecting their output alone does not suppress console allocation. GUI
//! applications (Explorer, the editor itself) use their normal launch paths.
use std::{ffi::OsStr, process::Command};

pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    tracing::debug!(program = ?program.as_ref(), "preparing background helper");
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW; retain redirected pipes.
    }
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{io::Write, process::Stdio};

    #[test]
    fn background_helpers_have_no_console_and_keep_stdio_and_exit_status() {
        // Check the actual console handle, rather than just our builder flags.
        let script = include_str!("../../../scripts/tests/windows-console-probe.ps1");
        let mut child = background_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"helper stdin\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
            "0\nhelper stdin\n"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("helper stderr"));
    }
}
