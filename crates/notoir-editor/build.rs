//! Compiles the syntax set (two-face's bat grammars plus
//! `assets/syntaxes/*.sublime-syntax`) into one packdump so the app never
//! parses grammars at startup (docs/specs/26).

use std::path::PathBuf;

fn main() {
    let dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/syntaxes");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut builder = two_face::syntax::extra_newlines().into_builder();
    builder
        .add_from_folder(&dir, true)
        .unwrap_or_else(|e| panic!("bad syntax definition in {}: {e}", dir.display()));
    let set = builder.build();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("syntaxes.packdump");
    syntect::dumps::dump_to_uncompressed_file(&set, &out).expect("write packdump");
}
