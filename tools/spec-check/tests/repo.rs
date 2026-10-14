//! Keeps spec status, test tags and the generated coverage report consistent.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn specs_and_tags_are_consistent() {
    let specs = spec_check::load_specs(&repo().join(spec_check::SPECS_DIR)).unwrap();
    let tags = spec_check::load_tags(&repo(), spec_check::TAG_ROOTS).unwrap();
    let problems = spec_check::check(&specs, &tags);
    assert!(
        problems.is_empty(),
        "spec traceability problems:\n{}",
        problems.join("\n")
    );
}

#[test]
fn coverage_report_is_up_to_date() {
    let specs = spec_check::load_specs(&repo().join(spec_check::SPECS_DIR)).unwrap();
    let tags = spec_check::load_tags(&repo(), spec_check::TAG_ROOTS).unwrap();
    let expected = spec_check::render_report(&specs, &tags);
    let actual = std::fs::read_to_string(repo().join(spec_check::REPORT)).unwrap_or_default();
    assert!(
        actual.replace("\r\n", "\n") == expected,
        "{} is stale: run `cargo run -p spec-check -- --write`",
        spec_check::REPORT
    );
}
