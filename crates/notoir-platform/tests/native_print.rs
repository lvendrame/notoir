//! Native PDF layout regressions and Windows renderer compilation.
//! No dialog or physical printer is opened; all output is temporary.

use std::process::Command;

#[cfg(target_os = "macos")]
#[test]
fn native_pdf_paginates_unicode_wraps_lines_and_respects_page_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("print-layout-test");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/print_test.m");
    let compile = Command::new("clang")
        .args([
            "-fobjc-arc",
            "-fblocks",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-framework",
            "Cocoa",
            "-framework",
            "PDFKit",
        ])
        .arg(source)
        .arg("-o")
        .arg(&helper)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let output = Command::new(helper).arg(dir.path()).output().unwrap();
    assert!(
        output.status.success(),
        "native PDF check exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "windows")]
#[test]
fn windows_native_dialog_and_renderer_compile() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/print.ps1");
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-CompileOnly")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a desktop display; run with xvfb-run -a cargo test -p notoir-platform --test native_print -- --ignored"]
fn gtk_native_pdf_paginates_and_prints_selection() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("print-layout-test");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/print_test.c");
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "gtk+-3.0"])
        .output()
        .unwrap();
    assert!(flags.status.success());
    let flags = String::from_utf8(flags.stdout).unwrap();
    let compile = Command::new("cc")
        .args(["-Wall", "-Wextra", "-Werror"])
        .arg(source)
        .arg("-o")
        .arg(&helper)
        .args(flags.split_whitespace())
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let output = Command::new(helper).arg(dir.path()).output().unwrap();
    assert!(
        output.status.success(),
        "native GTK PDF check exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}
