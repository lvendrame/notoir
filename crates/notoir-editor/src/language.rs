//! Language identification (docs/specs/26-syntax-outline-and-breadcrumbs.md,
//! 02-document-model.md §Loading step 6).

/// Stable language id (`"markdown"`, `"json"`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LanguageId(pub &'static str);

impl LanguageId {
    pub const PLAINTEXT: LanguageId = LanguageId("plaintext");
    pub const MARKDOWN: LanguageId = LanguageId("markdown");
    pub const JSON: LanguageId = LanguageId("json");
    pub const JSONC: LanguageId = LanguageId("jsonc");

    pub fn as_str(&self) -> &'static str {
        self.0
    }

    /// Looks up a registered language by id.
    pub fn from_id(id: &str) -> Option<LanguageId> {
        LANGUAGES.iter().find(|l| l.id.0 == id).map(|l| l.id)
    }
}

/// Static description of a language.
#[derive(Debug)]
pub struct LanguageDef {
    pub id: LanguageId,
    /// Display name (proper nouns are not translated).
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub file_names: &'static [&'static str],
    /// Interpreter names matched in a `#!` first line.
    pub interpreters: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
}

macro_rules! lang {
    ($id:literal, $name:literal, [$($ext:literal),*], [$($file:literal),*], [$($interp:literal),*], $line:expr, $block:expr) => {
        LanguageDef {
            id: LanguageId($id),
            name: $name,
            extensions: &[$($ext),*],
            file_names: &[$($file),*],
            interpreters: &[$($interp),*],
            line_comment: $line,
            block_comment: $block,
        }
    };
}

pub static LANGUAGES: &[LanguageDef] = &[
    lang!(
        "plaintext",
        "Plain Text",
        ["txt", "text"],
        [],
        [],
        None,
        None
    ),
    lang!(
        "markdown",
        "Markdown",
        ["md", "markdown", "mdown", "mkd", "mdx"],
        [],
        [],
        None,
        Some(("<!--", "-->"))
    ),
    lang!(
        "json",
        "JSON",
        ["json", "geojson", "webmanifest"],
        [".prettierrc", "composer.lock"],
        [],
        None,
        None
    ),
    lang!(
        "jsonc",
        "JSON with Comments",
        ["jsonc", "json5"],
        [
            "tsconfig.json",
            "jsconfig.json",
            ".eslintrc.json",
            "settings.json",
            "devcontainer.json"
        ],
        [],
        Some("//"),
        Some(("/*", "*/"))
    ),
    lang!(
        "jsonl",
        "JSON Lines",
        ["jsonl", "ndjson"],
        [],
        [],
        None,
        None
    ),
    lang!("yaml", "YAML", ["yaml", "yml"], [], [], Some("#"), None),
    lang!(
        "toml",
        "TOML",
        ["toml"],
        ["Cargo.lock", "Pipfile"],
        [],
        Some("#"),
        None
    ),
    lang!(
        "xml",
        "XML",
        ["xml", "xsd", "xsl", "xslt", "svg", "plist", "csproj"],
        [],
        [],
        None,
        Some(("<!--", "-->"))
    ),
    lang!(
        "html",
        "HTML",
        ["html", "htm", "xhtml"],
        [],
        [],
        None,
        Some(("<!--", "-->"))
    ),
    lang!(
        "css",
        "CSS",
        ["css", "scss", "less"],
        [],
        [],
        None,
        Some(("/*", "*/"))
    ),
    lang!(
        "javascript",
        "JavaScript",
        ["js", "mjs", "cjs", "jsx"],
        [],
        ["node", "deno", "bun"],
        Some("//"),
        Some(("/*", "*/"))
    ),
    lang!(
        "typescript",
        "TypeScript",
        ["ts", "mts", "cts", "tsx"],
        [],
        [],
        Some("//"),
        Some(("/*", "*/"))
    ),
    lang!(
        "rust",
        "Rust",
        ["rs"],
        [],
        [],
        Some("//"),
        Some(("/*", "*/"))
    ),
    lang!(
        "python",
        "Python",
        ["py", "pyw", "pyi"],
        [],
        ["python", "python3", "python2"],
        Some("#"),
        None
    ),
    lang!(
        "shellscript",
        "Shell Script",
        ["sh", "bash", "zsh", "ksh", "fish"],
        [".bashrc", ".zshrc", ".profile", ".bash_profile"],
        ["sh", "bash", "zsh", "ksh", "fish", "dash"],
        Some("#"),
        None
    ),
    lang!(
        "powershell",
        "PowerShell",
        ["ps1", "psm1", "psd1"],
        [],
        ["pwsh", "powershell"],
        Some("#"),
        Some(("<#", "#>"))
    ),
    lang!(
        "sql",
        "SQL",
        ["sql"],
        [],
        [],
        Some("--"),
        Some(("/*", "*/"))
    ),
    lang!(
        "ini",
        "INI",
        ["ini", "cfg", "conf", "properties"],
        [".editorconfig", ".gitconfig"],
        [],
        Some(";"),
        None
    ),
    lang!("dotenv", "Dotenv", ["env"], [".env"], [], Some("#"), None),
    lang!("diff", "Diff", ["diff", "patch"], [], [], None, None),
    lang!(
        "gitcommit",
        "Git Commit Message",
        [],
        ["COMMIT_EDITMSG", "MERGE_MSG", "TAG_EDITMSG"],
        [],
        Some("#"),
        None
    ),
    lang!("log", "Log", ["log"], [], [], None, None),
];

pub fn definition(id: LanguageId) -> &'static LanguageDef {
    LANGUAGES
        .iter()
        .find(|l| l.id == id)
        .unwrap_or(&LANGUAGES[0])
}

/// One `files.associations` entry: glob → language id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Association {
    pub pattern: String,
    pub language: String,
}

/// Detection order: associations (workspace before user, caller passes them in
/// priority order) → file name → extension → `#!` / first-line heuristics →
/// plaintext. A user's explicit override is handled by the caller.
pub fn detect(path: Option<&str>, first_line: &str, associations: &[Association]) -> LanguageId {
    if let Some(path) = path {
        let path = path.replace('\\', "/");
        let name = path.rsplit('/').next().unwrap_or(&path);
        for a in associations {
            let subject = if a.pattern.contains('/') {
                path.as_str()
            } else {
                name
            };
            if glob_match(&a.pattern, subject)
                && let Some(id) = LanguageId::from_id(&a.language)
            {
                return id;
            }
        }
        if let Some(l) = LANGUAGES.iter().find(|l| l.file_names.contains(&name)) {
            return l.id;
        }
        if let Some((_, ext)) = name.rsplit_once('.') {
            let ext = ext.to_ascii_lowercase();
            if let Some(l) = LANGUAGES
                .iter()
                .find(|l| l.extensions.contains(&ext.as_str()))
            {
                return l.id;
            }
        }
    }
    detect_first_line(first_line).unwrap_or(LanguageId::PLAINTEXT)
}

fn detect_first_line(line: &str) -> Option<LanguageId> {
    let line = line.trim_start_matches('\u{feff}').trim();
    if let Some(shebang) = line.strip_prefix("#!") {
        let mut words = shebang.split_whitespace();
        let first = words.next()?;
        let program = if first.ends_with("/env") {
            words.find(|w| !w.starts_with('-'))?
        } else {
            first.rsplit('/').next()?
        };
        let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        return LANGUAGES
            .iter()
            .find(|l| {
                l.interpreters
                    .iter()
                    .any(|i| i.trim_end_matches(|c: char| c.is_ascii_digit()) == program)
            })
            .map(|l| l.id);
    }
    if line.starts_with("<?xml") {
        return Some(LanguageId("xml"));
    }
    if line.starts_with("<!DOCTYPE html") || line.starts_with("<html") {
        return Some(LanguageId("html"));
    }
    None
}

/// Minimal glob: `*` matches within a path segment, `**` across segments,
/// `?` one char. Case-sensitive.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => {
                let rest = if p.get(2) == Some(&'/') {
                    &p[3..]
                } else {
                    &p[2..]
                };
                (0..=t.len()).any(|i| go(rest, &t[i..]))
            }
            Some('*') => (0..=t.len())
                .take_while(|&i| i == 0 || t[i - 1] != '/')
                .any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && t[0] != '/' && go(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_and_file_name() {
        assert_eq!(detect(Some("/a/README.md"), "", &[]), LanguageId::MARKDOWN);
        assert_eq!(
            detect(Some("C:\\x\\Cargo.lock"), "", &[]),
            LanguageId("toml")
        );
        assert_eq!(detect(Some("/a/tsconfig.json"), "", &[]), LanguageId::JSONC);
        assert_eq!(detect(Some("/a/config.JSON"), "", &[]), LanguageId::JSON);
        assert_eq!(detect(Some("/a/notes"), "", &[]), LanguageId::PLAINTEXT);
    }

    #[test]
    fn shebang_and_first_line() {
        assert_eq!(
            detect(Some("/bin/tool"), "#!/usr/bin/env python3", &[]),
            LanguageId("python")
        );
        assert_eq!(
            detect(None, "#!/bin/bash -e", &[]),
            LanguageId("shellscript")
        );
        assert_eq!(
            detect(None, "<?xml version=\"1.0\"?>", &[]),
            LanguageId("xml")
        );
    }

    #[test]
    fn associations_win() {
        let assoc = [
            Association {
                pattern: "*.txt".into(),
                language: "markdown".into(),
            },
            Association {
                pattern: "**/.vscode/*.json".into(),
                language: "jsonc".into(),
            },
        ];
        assert_eq!(
            detect(Some("/n/todo.txt"), "", &assoc),
            LanguageId::MARKDOWN
        );
        assert_eq!(
            detect(Some("/p/.vscode/launch.json"), "", &assoc),
            LanguageId::JSONC
        );
        assert_eq!(detect(Some("/p/launch.json"), "", &assoc), LanguageId::JSON);
    }

    #[test]
    fn glob() {
        assert!(glob_match("*.md", "a.md"));
        assert!(!glob_match("*.md", "dir/a.md"));
        assert!(glob_match("**/a.md", "x/y/a.md"));
        assert!(glob_match("**/a.md", "a.md"));
        assert!(glob_match("file?.txt", "file1.txt"));
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in LANGUAGES.iter().enumerate() {
            assert!(
                LANGUAGES[i + 1..].iter().all(|b| b.id != a.id),
                "{:?}",
                a.id
            );
        }
    }
}
