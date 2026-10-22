//! Embeds every `assets/locales/<lang>/<ns>.json` as a static table (spec 22).

use std::fmt::Write;
use std::path::Path;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_finder_events();
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/locales");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut entries = Vec::new();
    for lang in std::fs::read_dir(&root).expect("locales dir") {
        let lang = lang.expect("entry").path();
        if !lang.is_dir() {
            continue;
        }
        for ns in std::fs::read_dir(&lang).expect("lang dir") {
            let ns = ns.expect("entry").path();
            if ns.extension().is_some_and(|e| e == "json") {
                println!("cargo:rerun-if-changed={}", ns.display());
                entries.push((
                    lang.file_name().unwrap().to_string_lossy().into_owned(),
                    ns.file_stem().unwrap().to_string_lossy().into_owned(),
                    ns.canonicalize().unwrap(),
                ));
            }
        }
    }
    entries.sort();
    let mut out = String::from("pub static LOCALE_FILES: &[(&str, &str, &str)] = &[\n");
    for (lang, ns, path) in entries {
        writeln!(
            out,
            "    ({lang:?}, {ns:?}, include_str!({:?})),",
            path.display().to_string()
        )
        .unwrap();
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("locales.rs");
    std::fs::write(dest, out).unwrap();
}

// Keep Objective-C interop out of Rust: the workspace forbids unsafe Rust.
fn build_finder_events() {
    use std::process::Command;
    println!("cargo:rerun-if-changed=native/finder.m");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let object = out.join("finder.o");
    let library = out.join("libnotoir_finder.a");
    let sdk = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .expect("macOS SDK");
    assert!(sdk.status.success(), "macOS SDK lookup failed");
    let sdk = String::from_utf8(sdk.stdout).unwrap();
    let status = Command::new("clang")
        .args([
            "-target",
            &std::env::var("TARGET").unwrap(),
            "-isysroot",
            sdk.trim(),
            "-mmacosx-version-min=11.0",
            "-fobjc-arc",
            "-fblocks",
            "-c",
            "native/finder.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("compile Finder event handler");
    assert!(status.success(), "Finder event handler compilation failed");
    assert!(
        Command::new("ar")
            .arg("-crs")
            .arg(&library)
            .arg(&object)
            .status()
            .expect("archive Finder event handler")
            .success()
    );
    println!(
        "cargo:rustc-link-arg-bin=notoir=-Wl,-force_load,{}",
        library.display()
    );
    println!("cargo:rustc-link-lib=framework=Cocoa");
}
