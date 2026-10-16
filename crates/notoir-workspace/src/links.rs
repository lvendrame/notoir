//! Link and tag index of a workspace's Markdown notes (docs/specs/34): wiki
//! links with their line context, tags, link resolution and backlinks. A
//! cache built from the files (and open buffers); it can always be rebuilt.

use std::collections::BTreeMap;
use std::path::Path;

use notoir_editor::wiki;

/// File extensions indexed as notes.
pub const NOTE_EXTENSIONS: &[&str] = &["md", "markdown"];

pub fn is_note(rel: &str) -> bool {
    rel.rsplit_once('.')
        .is_some_and(|(_, ext)| NOTE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The file name without its extension (`notes/Plan.md` → `Plan`).
pub fn stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedLink {
    pub target: String,
    pub heading: Option<String>,
    /// 0-based line.
    pub line: usize,
    /// The line's text (trimmed, shortened) for context.
    pub snippet: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteEntry {
    pub links: Vec<IndexedLink>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Workspace-relative path.
    Found(String),
    Ambiguous(Vec<String>),
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backlink {
    pub source: String,
    pub line: usize,
    pub snippet: String,
}

#[derive(Clone, Debug, Default)]
pub struct LinkIndex {
    /// Every workspace file (link resolution also finds non-notes).
    files: Vec<String>,
    /// Notes by relative path.
    notes: BTreeMap<String, NoteEntry>,
}

const SNIPPET_CHARS: usize = 120;

fn snippet(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() <= SNIPPET_CHARS {
        t.to_owned()
    } else {
        let mut s: String = t.chars().take(SNIPPET_CHARS).collect();
        s.push('…');
        s
    }
}

/// Parses one note.
pub fn parse_note(text: &str) -> NoteEntry {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(
            text.chars()
                .enumerate()
                .filter(|(_, c)| *c == '\n')
                .map(|(i, _)| i + 1),
        )
        .collect();
    let lines: Vec<&str> = text.split('\n').collect();
    let line_of = |at: usize| line_starts.partition_point(|s| *s <= at).saturating_sub(1);
    let links = wiki::links(text)
        .into_iter()
        .map(|l| {
            let line = line_of(l.range.0);
            IndexedLink {
                target: l.target,
                heading: l.heading,
                line,
                snippet: snippet(lines.get(line).copied().unwrap_or_default()),
            }
        })
        .collect();
    let mut tags: Vec<String> = wiki::tags(text).into_iter().map(|t| t.name).collect();
    tags.sort_by_key(|t| t.to_lowercase());
    tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    NoteEntry { links, tags }
}

impl LinkIndex {
    /// Builds the index: `files` are all workspace files (relative, `/`),
    /// notes among them are read from `root`.
    pub fn build(root: &Path, files: Vec<String>) -> LinkIndex {
        let mut notes = BTreeMap::new();
        for rel in files.iter().filter(|r| is_note(r)) {
            if let Ok(text) = std::fs::read_to_string(root.join(rel)) {
                notes.insert(rel.clone(), parse_note(&text));
            }
        }
        LinkIndex { files, notes }
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    /// Replaces a note's entry (a saved file or an edited buffer).
    pub fn update(&mut self, rel: &str, text: &str) {
        if !self.files.iter().any(|f| f == rel) {
            self.files.push(rel.to_owned());
        }
        self.notes.insert(rel.to_owned(), parse_note(text));
    }

    pub fn remove(&mut self, rel: &str) {
        self.files.retain(|f| f != rel);
        self.notes.remove(rel);
    }

    pub fn note(&self, rel: &str) -> Option<&NoteEntry> {
        self.notes.get(rel)
    }

    /// Resolves a link target written in `from` (case-insensitive file stem
    /// or name, then a path relative to `from`'s folder or the root).
    pub fn resolve(&self, target: &str, from: Option<&str>) -> Resolution {
        let target = target.trim().trim_start_matches("./");
        if target.is_empty() {
            return Resolution::Missing;
        }
        let lower = target.to_lowercase();
        let with_md = |p: &str| -> Vec<String> {
            let p = p.to_lowercase();
            self.files
                .iter()
                .filter(|f| {
                    let f = f.to_lowercase();
                    f == p || NOTE_EXTENSIONS.iter().any(|e| f == format!("{p}.{e}"))
                })
                .cloned()
                .collect()
        };
        if target.contains('/') {
            let base = from
                .and_then(|f| f.rsplit_once('/'))
                .map(|(dir, _)| format!("{dir}/{target}"));
            for candidate in base.iter().map(String::as_str).chain([target]) {
                let found = with_md(&normalize(candidate));
                if let Some(f) = found.into_iter().next() {
                    return Resolution::Found(f);
                }
            }
            return Resolution::Missing;
        }
        let mut hits: Vec<String> = self
            .files
            .iter()
            .filter(|f| {
                let name = f.rsplit('/').next().unwrap_or(f).to_lowercase();
                (is_note(f) && stem(f).to_lowercase() == lower) || name == lower
            })
            .cloned()
            .collect();
        hits.sort();
        match hits.len() {
            0 => Resolution::Missing,
            1 => Resolution::Found(hits.remove(0)),
            _ => {
                // Prefer a note next to the linking file.
                let dir = from.and_then(|f| f.rsplit_once('/')).map(|(d, _)| d);
                let near: Vec<&String> = hits
                    .iter()
                    .filter(|h| h.rsplit_once('/').map(|(d, _)| d) == dir)
                    .collect();
                match near.as_slice() {
                    [one] => Resolution::Found((*one).clone()),
                    _ => Resolution::Ambiguous(hits),
                }
            }
        }
    }

    /// Notes linking to `rel`.
    pub fn backlinks(&self, rel: &str) -> Vec<Backlink> {
        let mut out = Vec::new();
        for (source, note) in &self.notes {
            if source == rel {
                continue;
            }
            for l in &note.links {
                if self.resolve(&l.target, Some(source)) == Resolution::Found(rel.to_owned()) {
                    out.push(Backlink {
                        source: source.clone(),
                        line: l.line,
                        snippet: l.snippet.clone(),
                    });
                }
            }
        }
        out
    }

    /// Tags with the number of notes using them, by name.
    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for note in self.notes.values() {
            for t in &note.tags {
                let e = counts
                    .entry(t.to_lowercase())
                    .or_insert_with(|| (t.clone(), 0));
                e.1 += 1;
            }
        }
        counts.into_values().collect()
    }

    pub fn files_with_tag(&self, tag: &str) -> Vec<String> {
        self.notes
            .iter()
            .filter(|(_, n)| n.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
            .map(|(rel, _)| rel.clone())
            .collect()
    }

    /// Notes for `[[` completion: (stem, relative path).
    pub fn note_names(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|f| is_note(f))
            .map(|f| (stem(f).to_owned(), f.clone()))
            .collect()
    }

    /// Notes with links to `rel` (rename: the files to update).
    pub fn linking_notes(&self, rel: &str) -> Vec<String> {
        let mut v: Vec<String> = self.backlinks(rel).into_iter().map(|b| b.source).collect();
        v.dedup();
        v
    }
}

/// Resolves `.` and `..` in a relative path.
fn normalize(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> LinkIndex {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let files = [
            (
                "Architecture Decisions.md",
                "# AD\n\n## MCP\n#architecture\n",
            ),
            (
                "Project Plan.md",
                "We decided in [[Architecture Decisions]].\n#plan #rust\n",
            ),
            (
                "notes/MCP Architecture.md",
                "See [[architecture decisions#MCP|MCP]] and [[../Project Plan]].\n",
            ),
            ("notes/Plan.md", "local [[Project Plan]]\n"),
            ("a/Dup.md", ""),
            ("b/Dup.md", "[[Dup]]"),
            ("diagram.png", ""),
        ];
        let mut rels = Vec::new();
        for (rel, text) in files {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
            rels.push(rel.to_owned());
        }
        LinkIndex::build(root, rels)
    }

    #[test]
    fn resolution() {
        let i = index();
        assert_eq!(
            i.resolve("architecture decisions", None),
            Resolution::Found("Architecture Decisions.md".into())
        );
        assert_eq!(
            i.resolve("../Project Plan", Some("notes/MCP Architecture.md")),
            Resolution::Found("Project Plan.md".into())
        );
        assert_eq!(
            i.resolve("diagram.png", None),
            Resolution::Found("diagram.png".into())
        );
        assert_eq!(i.resolve("Nope", None), Resolution::Missing);
        // Two notes named Dup: the one next to the source wins, else ambiguous.
        assert_eq!(
            i.resolve("Dup", Some("b/Dup.md")),
            Resolution::Found("b/Dup.md".into())
        );
        assert_eq!(
            i.resolve("Dup", Some("Project Plan.md")),
            Resolution::Ambiguous(vec!["a/Dup.md".into(), "b/Dup.md".into()])
        );
    }

    #[test]
    fn backlinks_tags_and_updates() {
        let mut i = index();
        let back: Vec<String> = i
            .backlinks("Architecture Decisions.md")
            .into_iter()
            .map(|b| format!("{}:{} {}", b.source, b.line, b.snippet))
            .collect();
        assert_eq!(
            back,
            [
                "Project Plan.md:0 We decided in [[Architecture Decisions]].",
                "notes/MCP Architecture.md:0 See [[architecture decisions#MCP|MCP]] and [[../Project Plan]]."
            ]
        );
        assert_eq!(
            i.tags(),
            [
                ("architecture".into(), 1),
                ("plan".into(), 1),
                ("rust".into(), 1)
            ]
        );
        assert_eq!(i.files_with_tag("RUST"), ["Project Plan.md"]);
        // An edited buffer replaces the entry.
        i.update("Project Plan.md", "no links now #rust");
        assert_eq!(i.backlinks("Architecture Decisions.md").len(), 1);
        i.remove("notes/MCP Architecture.md");
        assert!(i.backlinks("Architecture Decisions.md").is_empty());
        assert!(i.note_names().iter().any(|(s, _)| s == "Project Plan"));
    }
}
