//! The user guide (`docs/guide/`) stays in sync with the app: generated
//! reference pages match the registries, every chapter in `SUMMARY.md` exists,
//! and every screenshot is both present and used.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn guide() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide/src")
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// Targets of Markdown links and images (`](target)`), without anchors.
fn link_targets(text: &str) -> Vec<String> {
    text.match_indices("](")
        .filter_map(|(i, _)| {
            let rest = &text[i + 2..];
            let end = rest.find(')')?;
            let target = rest[..end].split('#').next()?.trim();
            (!target.is_empty() && !target.contains("://") && !target.starts_with("mailto:"))
                .then(|| target.to_owned())
        })
        .collect()
}

#[test]
fn reference_pages_are_current() {
    let dir = guide().join("reference");
    for (name, text) in notoir::docs_ref::render_all() {
        let committed = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        assert!(
            committed.replace("\r\n", "\n") == text,
            "docs/guide/src/reference/{name} is stale: run `cargo run -p notoir --example gen_docs`"
        );
    }
}

/// Every title in the generated pages is translated (no raw `ns:key`).
#[test]
fn reference_titles_are_translated() {
    for (name, text) in notoir::docs_ref::render_all() {
        for ns in ["commands:", "settings:", "menus:", "mcp:"] {
            assert!(
                !text.contains(&format!("| {ns}")) && !text.contains(&format!("- {ns}")),
                "{name} shows an untranslated {ns} key"
            );
        }
    }
}

#[test]
fn links_and_images_resolve() {
    let root = guide();
    let mut files = Vec::new();
    markdown_files(&root, &mut files);
    let mut used = BTreeSet::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let base = file.parent().unwrap();
        for target in link_targets(&text) {
            let path = base.join(&target);
            assert!(
                path.exists(),
                "{} links to missing {target}",
                file.strip_prefix(&root).unwrap().display()
            );
            if let Ok(p) = path.canonicalize() {
                used.insert(p);
            }
        }
    }
    let images = root.join("images");
    let mut stack = vec![images];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                assert!(
                    used.contains(&path.canonicalize().unwrap()),
                    "unused image {}",
                    path.strip_prefix(&root).unwrap().display()
                );
            }
        }
    }
}

#[test]
fn every_page_is_in_the_summary() {
    let root = guide();
    let summary = std::fs::read_to_string(root.join("SUMMARY.md")).unwrap();
    let listed: BTreeSet<PathBuf> = link_targets(&summary)
        .into_iter()
        .map(|t| root.join(t))
        .collect();
    let mut files = Vec::new();
    markdown_files(&root, &mut files);
    for file in files {
        if file.file_name().is_some_and(|n| n == "SUMMARY.md") {
            continue;
        }
        assert!(
            listed.contains(&file),
            "{} is not listed in SUMMARY.md",
            file.strip_prefix(&root).unwrap().display()
        );
    }
}
