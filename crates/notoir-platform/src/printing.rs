//! Native printing runs in a child of the signed application executable. Each
//! process owns its native main thread; the editor never waits on a modal OS
//! dialog. Snapshot files live in a private temporary directory until it exits.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintRequest {
    pub title: String,
    pub text: String,
    pub selection: Option<String>,
    pub selection_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintOutcome {
    Cancelled,
    Submitted,
}

/// Runs on a dedicated worker, never on the application's UI or save queue.
pub trait Printer: Send + Sync {
    fn print(&self, request: PrintRequest) -> Result<PrintOutcome, String>;
}

#[derive(Default)]
pub struct NativePrinter;

impl Printer for NativePrinter {
    fn print(&self, request: PrintRequest) -> Result<PrintOutcome, String> {
        native_print(request).map_err(|e| e.to_string())
    }
}

fn native_print(request: PrintRequest) -> std::io::Result<PrintOutcome> {
    #[cfg(not(target_os = "windows"))]
    use std::process::Command;
    let dir = tempfile::Builder::new().prefix("notoir-print-").tempdir()?;
    // Do not pass document text through shell arguments, logs or shared paths.
    for (name, text) in [
        ("title", request.title.as_str()),
        ("text", request.text.as_str()),
        ("selection-label", request.selection_label.as_str()),
    ] {
        std::fs::write(dir.path().join(name), printable_text(text))?;
    }
    if let Some(selection) = request.selection {
        std::fs::write(dir.path().join("selection"), printable_text(&selection))?;
    }
    #[cfg(target_os = "windows")]
    let mut command = {
        let script = dir.path().join("print.ps1");
        std::fs::write(&script, include_str!("../native/print.ps1"))?;
        let mut c = crate::process::background_command("powershell.exe");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg(dir.path());
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut c = Command::new(std::env::current_exe()?);
        c.arg("--notoir-internal-print").arg(dir.path());
        c
    };
    let output = command.output()?;
    match output.status.code() {
        Some(0) => Ok(PrintOutcome::Submitted),
        Some(2) => Ok(PrintOutcome::Cancelled),
        _ => Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        )),
    }
}

/// Native text APIs use NUL-terminated strings. Preserve all other Unicode and
/// normalize line endings for consistent pagination; make NUL visible, rather
/// than silently truncating the rest of the document.
pub fn printable_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "\u{2400}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalizes_line_endings_without_losing_unicode_or_blank_lines() {
        assert_eq!(
            super::printable_text("\t日本語 😀\r\n\r\na\0b\r"),
            "\t日本語 😀\n\na␀b\n"
        );
    }
}
