//! Spec status and scenario → test traceability.
//!
//! - Every spec in `docs/specs/` carries a status line right after its title:
//!   `**Status:** Done | Partial | Not started — <notes>`.
//! - A test covers a Gherkin scenario by a comment tag naming the spec number
//!   and the exact scenario title: `// spec 03: "Nested split layout"`.
//!   Tags in `apps/*/tests/` (egui_kittest) count as app-level coverage;
//!   everything else (crate and app unit tests) as domain-level.
//! - [`check`] reports stale tags, duplicate titles, missing/invalid status
//!   lines and status/coverage contradictions; [`render_report`] produces
//!   `docs/plan/spec-coverage.md`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Done,
    Partial,
    NotStarted,
}

impl StatusKind {
    fn label(self) -> &'static str {
        match self {
            StatusKind::Done => "Done",
            StatusKind::Partial => "Partial",
            StatusKind::NotStarted => "Not started",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub kind: StatusKind,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub number: String,
    pub file: String,
    pub title: String,
    pub status: Option<Status>,
    pub scenarios: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Domain,
    App,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub spec: String,
    pub scenario: String,
    /// Repo-relative path.
    pub file: String,
    pub line: usize,
    /// Name of the test function following the tag, if found.
    pub test: Option<String>,
    pub level: Level,
}

/// Repo-relative locations.
pub const SPECS_DIR: &str = "docs/specs";
pub const REPORT: &str = "docs/plan/spec-coverage.md";
pub const TAG_ROOTS: &[&str] = &["crates", "apps"];

const STATUS_PREFIX: &str = "**Status:** ";

fn parse_status(line: &str) -> Option<Status> {
    let rest = line.strip_prefix(STATUS_PREFIX)?;
    let (kind, note) = match rest.split_once(" — ") {
        Some((k, n)) => (k.trim(), n.trim()),
        None => (rest.trim(), ""),
    };
    let kind = match kind {
        "Done" => StatusKind::Done,
        "Partial" => StatusKind::Partial,
        "Not started" => StatusKind::NotStarted,
        _ => return None,
    };
    Some(Status {
        kind,
        note: note.to_owned(),
    })
}

/// Parses one spec file's text.
pub fn parse_spec(file: &str, text: &str) -> Spec {
    let number = file.split('-').next().unwrap_or_default().to_owned();
    let title = text
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| {
            t.split_once(" — ")
                .map_or(t, |(_, name)| name)
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    let status = text
        .lines()
        .find(|l| l.starts_with(STATUS_PREFIX))
        .and_then(parse_status);
    let scenarios = text
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("Scenario: "))
        .map(|s| s.trim().to_owned())
        .collect();
    Spec {
        number,
        file: file.to_owned(),
        title,
        status,
        scenarios,
    }
}

pub fn load_specs(specs_dir: &Path) -> std::io::Result<Vec<Spec>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(specs_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Ok(parse_spec(&name, &std::fs::read_to_string(p)?))
        })
        .collect()
}

/// Extracts `// spec NN: "title"` tags from one Rust source file.
pub fn parse_tags(rel_path: &str, text: &str) -> Vec<Tag> {
    let level = if rel_path.starts_with("apps/") && rel_path.contains("/tests/") {
        Level::App
    } else {
        Level::Domain
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(comment) = line.trim_start().strip_prefix("//") else {
            continue;
        };
        let Some(rest) = comment.trim_start_matches('/').trim().strip_prefix("spec ") else {
            continue;
        };
        let Some((number, quoted)) = rest.split_once(':') else {
            continue;
        };
        let number = number.trim();
        if number.len() != 2 || !number.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let quoted = quoted.trim();
        let Some(title) = quoted.strip_prefix('"').and_then(|q| q.strip_suffix('"')) else {
            continue;
        };
        let test = lines[i + 1..]
            .iter()
            .take(8)
            .find_map(|l| {
                l.trim_start()
                    .strip_prefix("fn ")
                    .or_else(|| l.trim_start().strip_prefix("async fn "))
            })
            .and_then(|l| l.split('(').next())
            .map(str::to_owned);
        out.push(Tag {
            spec: number.to_owned(),
            scenario: title.to_owned(),
            file: rel_path.to_owned(),
            line: i + 1,
            test,
            level,
        });
    }
    out
}

/// Scans `.rs` files under `roots` (relative to `repo`), skipping `target/`.
pub fn load_tags(repo: &Path, roots: &[&str]) -> std::io::Result<Vec<Tag>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    for root in roots {
        let dir = repo.join(root);
        if dir.is_dir() {
            walk(&dir, &mut files)?;
        }
    }
    files.sort();
    let mut tags = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(repo)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        tags.extend(parse_tags(&rel, &std::fs::read_to_string(&f)?));
    }
    Ok(tags)
}

/// Tags per (spec number, scenario title).
fn index(tags: &[Tag]) -> BTreeMap<(&str, &str), Vec<&Tag>> {
    let mut map: BTreeMap<(&str, &str), Vec<&Tag>> = BTreeMap::new();
    for t in tags {
        map.entry((t.spec.as_str(), t.scenario.as_str()))
            .or_default()
            .push(t);
    }
    map
}

/// Consistency problems; empty means the repo is consistent.
pub fn check(specs: &[Spec], tags: &[Tag]) -> Vec<String> {
    let mut problems = Vec::new();
    let idx = index(tags);
    for spec in specs {
        match &spec.status {
            None => problems.push(format!(
                "{}: missing or invalid status line (expected `{STATUS_PREFIX}Done|Partial|Not started — notes`)",
                spec.file
            )),
            Some(status) => {
                let covered = spec.scenarios.iter().filter(|s| idx.contains_key(&(spec.number.as_str(), s.as_str()))).count();
                if status.kind == StatusKind::Done && covered < spec.scenarios.len() {
                    problems.push(format!(
                        "{}: status is Done but {} of {} scenarios have no test",
                        spec.file,
                        spec.scenarios.len() - covered,
                        spec.scenarios.len()
                    ));
                }
                if status.kind == StatusKind::NotStarted && covered > 0 {
                    problems.push(format!("{}: status is Not started but {covered} scenarios are covered", spec.file));
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in &spec.scenarios {
            if !seen.insert(s) {
                problems.push(format!("{}: duplicate scenario title {s:?}", spec.file));
            }
        }
    }
    for t in tags {
        let known = specs
            .iter()
            .any(|s| s.number == t.spec && s.scenarios.contains(&t.scenario));
        if !known {
            problems.push(format!(
                "{}:{}: tag `spec {}: {:?}` matches no scenario (renamed or typo?)",
                t.file, t.line, t.spec, t.scenario
            ));
        }
    }
    problems
}

/// Markdown coverage report (`docs/plan/spec-coverage.md`).
pub fn render_report(specs: &[Spec], tags: &[Tag]) -> String {
    let idx = index(tags);
    let total: usize = specs.iter().map(|s| s.scenarios.len()).sum();
    let mut covered_total = 0;
    let mut by_level: BTreeMap<Level, usize> = BTreeMap::new();
    for spec in specs {
        for s in &spec.scenarios {
            if let Some(ts) = idx.get(&(spec.number.as_str(), s.as_str())) {
                covered_total += 1;
                let best = ts.iter().map(|t| t.level).max().unwrap_or(Level::Domain);
                *by_level.entry(best).or_default() += 1;
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "# Spec Coverage\n");
    let _ = writeln!(
        out,
        "Generated by `cargo run -p spec-check -- --write` from the status lines in `docs/specs/*.md` and the\n\
         `// spec NN: \"Scenario title\"` tags on tests. Do not edit by hand; `cargo test` fails when it is stale.\n"
    );
    let _ = writeln!(
        out,
        "**{covered_total} / {total} scenarios covered** — app-level: {}, domain-level only: {}.\n",
        by_level.get(&Level::App).copied().unwrap_or(0),
        by_level.get(&Level::Domain).copied().unwrap_or(0)
    );
    let _ = writeln!(
        out,
        "*Domain-level* means the behaviour is verified by unit/integration tests below the UI; the UI part of the scenario\n\
         is not exercised yet. *App-level* means a UI test in `apps/*/tests/` (egui_kittest) covers it.\n"
    );
    let _ = writeln!(out, "| Spec | Status | Covered | Notes |");
    let _ = writeln!(out, "|---|---|---|---|");
    for spec in specs {
        let covered = spec
            .scenarios
            .iter()
            .filter(|s| idx.contains_key(&(spec.number.as_str(), s.as_str())))
            .count();
        let (status, note) = spec
            .status
            .as_ref()
            .map_or(("?", ""), |s| (s.kind.label(), s.note.as_str()));
        let _ = writeln!(
            out,
            "| [{} {}](../specs/{}) | {} | {}/{} | {} |",
            spec.number,
            spec.title,
            spec.file,
            status,
            covered,
            spec.scenarios.len(),
            note
        );
    }
    for spec in specs.iter().filter(|s| !s.scenarios.is_empty()) {
        let status = spec.status.as_ref().map_or("?", |s| s.kind.label());
        let _ = writeln!(out, "\n## {} — {} ({status})\n", spec.number, spec.title);
        for s in &spec.scenarios {
            match idx.get(&(spec.number.as_str(), s.as_str())) {
                Some(ts) => {
                    let refs: Vec<String> = ts
                        .iter()
                        .map(|t| {
                            let level = match t.level {
                                Level::App => "app",
                                Level::Domain => "domain",
                            };
                            match &t.test {
                                Some(name) => format!("{level}: `{}` `{name}`", t.file),
                                None => format!("{level}: `{}`", t.file),
                            }
                        })
                        .collect();
                    let _ = writeln!(out, "- [x] {s} — {}", refs.join("; "));
                }
                None => {
                    let _ = writeln!(out, "- [ ] {s}");
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "# 03 — Editor Views\n\n**Status:** Partial — domain (M1); UI M6\n\n```gherkin\nFeature: X\n\n  Scenario: Nested split layout\n    Given a\n\n  Scenario: Maximize and restore\n```\n";

    #[test]
    fn parses_spec_status_and_scenarios() {
        let s = parse_spec("03-editor-views.md", SPEC);
        assert_eq!(s.number, "03");
        assert_eq!(s.title, "Editor Views");
        assert_eq!(
            s.status,
            Some(Status {
                kind: StatusKind::Partial,
                note: "domain (M1); UI M6".into()
            })
        );
        assert_eq!(s.scenarios, ["Nested split layout", "Maximize and restore"]);
    }

    #[test]
    fn parses_tags_with_test_names_and_levels() {
        let src = "#[test]\n// spec 03: \"Nested split layout\"\n/// spec 03: \"Maximize and restore\"\nfn nested() {}\n// not a tag: spec 3: \"x\"\n";
        let tags = parse_tags("crates/notoir-core/src/layout.rs", src);
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].test.as_deref(), Some("nested"));
        assert_eq!(tags[0].level, Level::Domain);
        assert_eq!(
            parse_tags("apps/notoir/tests/x.rs", src)[0].level,
            Level::App
        );
        assert_eq!(
            parse_tags("apps/notoir/src/theme.rs", src)[0].level,
            Level::Domain
        );
    }

    #[test]
    fn check_finds_stale_tags_and_contradictions() {
        let mut spec = parse_spec("03-editor-views.md", SPEC);
        let tags = parse_tags(
            "crates/a.rs",
            "// spec 03: \"Nested split layout\"\nfn t() {}\n// spec 03: \"Renamed scenario\"\nfn u() {}\n",
        );
        let problems = check(std::slice::from_ref(&spec), &tags);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("matches no scenario"));

        spec.status = Some(Status {
            kind: StatusKind::Done,
            note: String::new(),
        });
        assert!(
            check(std::slice::from_ref(&spec), &tags[..1])
                .iter()
                .any(|p| p.contains("status is Done"))
        );
        spec.status = Some(Status {
            kind: StatusKind::NotStarted,
            note: String::new(),
        });
        assert!(
            check(std::slice::from_ref(&spec), &tags[..1])
                .iter()
                .any(|p| p.contains("Not started"))
        );
        spec.status = None;
        assert!(
            check(std::slice::from_ref(&spec), &[])
                .iter()
                .any(|p| p.contains("missing or invalid status"))
        );
    }

    #[test]
    fn report_marks_coverage() {
        let spec = parse_spec("03-editor-views.md", SPEC);
        let tags = parse_tags(
            "apps/notoir/tests/x.rs",
            "// spec 03: \"Nested split layout\"\nfn t() {}\n",
        );
        let r = render_report(&[spec], &tags);
        assert!(r.contains("**1 / 2 scenarios covered** — app-level: 1, domain-level only: 0."));
        assert!(r.contains("- [x] Nested split layout — app: `apps/notoir/tests/x.rs` `t`"));
        assert!(r.contains("- [ ] Maximize and restore"));
    }
}
