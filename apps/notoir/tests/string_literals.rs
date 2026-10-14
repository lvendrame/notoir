//! i18n discipline (spec 22): UI code never passes alphabetic string literals
//! straight to labels or buttons; text goes through `t!`.

use std::path::Path;

const CALLS: &[&str] = &[
    ".label(\"",
    ".button(\"",
    "RichText::new(\"",
    ".heading(\"",
    ".on_hover_text(\"",
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_untranslated_ui_literals() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut offenders = Vec::new();
    for file in files {
        let src = std::fs::read_to_string(&file).unwrap();
        for (n, line) in src.lines().enumerate() {
            for call in CALLS {
                let Some(start) = line.find(call) else {
                    continue;
                };
                let literal = &line[start + call.len()..];
                let text: String = literal.chars().take_while(|c| *c != '"').collect();
                if text.chars().any(char::is_alphabetic) {
                    offenders.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "untranslated UI text:\n{}",
        offenders.join("\n")
    );
}
