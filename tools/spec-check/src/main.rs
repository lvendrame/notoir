//! `cargo run -p spec-check` prints problems and a coverage summary;
//! `cargo run -p spec-check -- --write` regenerates `docs/plan/spec-coverage.md`.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let specs = spec_check::load_specs(&repo.join(spec_check::SPECS_DIR)).expect("read docs/specs");
    let tags = spec_check::load_tags(&repo, spec_check::TAG_ROOTS).expect("scan sources");
    let report = spec_check::render_report(&specs, &tags);
    if std::env::args().any(|a| a == "--write") {
        std::fs::write(repo.join(spec_check::REPORT), &report).expect("write report");
        println!("wrote {}", spec_check::REPORT);
    }
    let problems = spec_check::check(&specs, &tags);
    for p in &problems {
        eprintln!("problem: {p}");
    }
    if let Some(summary) = report.lines().find(|l| l.starts_with("**")) {
        println!("{summary}");
    }
    if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
