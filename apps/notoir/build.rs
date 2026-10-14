//! Embeds every `assets/locales/<lang>/<ns>.json` as a static table (spec 22).

use std::fmt::Write;
use std::path::Path;

fn main() {
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
