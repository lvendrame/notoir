//! Layer boundary rule (spec 01): domain crates never depend on UI or IO crates.

use std::path::Path;

const FORBIDDEN: &[&str] = &[
    "egui", "eframe", "tokio", "rusqlite", "notify", "git2", "rfd", "mlua", "rmcp",
];

fn dependencies(crate_dir: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(crate_dir)
        .join("Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(&path)
        .expect("manifest")
        .parse()
        .expect("valid toml");
    ["dependencies", "build-dependencies"]
        .iter()
        .filter_map(|section| manifest.get(*section).and_then(|v| v.as_table()))
        .flat_map(|t| t.keys().cloned())
        .collect()
}

#[test]
fn domain_crates_do_not_depend_on_ui_or_io_crates() {
    for crate_dir in ["notoir-core", "notoir-editor"] {
        for dep in dependencies(crate_dir) {
            assert!(
                !FORBIDDEN.contains(&dep.as_str()),
                "{crate_dir} must not depend on {dep}"
            );
        }
    }
}
