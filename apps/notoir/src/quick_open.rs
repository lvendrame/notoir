//! Quick Open sources and ranking (docs/specs/15 §Quick Open): workspace
//! index files, open documents and recent files, fuzzy matched on the
//! relative path; `file:line[:col]` suffixes; prefix providers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::document::DocumentOrigin;
use notoir_core::explorer::{IndexEntry, NodeKind};
use notoir_core::fuzzy::fuzzy_match;
use notoir_core::ids::DocumentId;

use crate::core::AppCore;

/// Rows shown at most.
pub const MAX_ROWS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Path(PathBuf),
    Doc(DocumentId),
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub target: Target,
    /// What is matched: root-relative path, absolute path, or title.
    pub key: String,
    /// Open documents and recent files rank higher.
    pub boost: i32,
}

/// Everything Quick Open can list, captured when it opens.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub root: Option<PathBuf>,
    pub index: Arc<Vec<IndexEntry>>,
    /// Open editors (MRU first) then recent files: the empty-query list.
    pub initial: Vec<Candidate>,
    pub indexing: bool,
    pub max_files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub target: Target,
    /// File name and the matched char positions in it.
    pub name: String,
    pub name_marks: Vec<usize>,
    /// Dimmed directory.
    pub dir: String,
}

/// Splits `README.md:120:4` → (`README.md`, Some((120, 4))).
pub fn split_line_suffix(q: &str) -> (&str, Option<(usize, usize)>) {
    let parts: Vec<&str> = q.rsplitn(3, ':').collect();
    let num = |s: &str| s.trim().parse::<usize>().ok().filter(|n| *n > 0);
    match parts.as_slice() {
        [c, l, rest] if num(c).is_some() && num(l).is_some() && !rest.is_empty() => {
            (rest, Some((num(l).unwrap_or(1), num(c).unwrap_or(1))))
        }
        [l, rest, ..] if num(l).is_some() && !rest.is_empty() => {
            let rest_len = q.len() - l.len() - 1;
            (&q[..rest_len], Some((num(l).unwrap_or(1), 1)))
        }
        _ => (q, None),
    }
}

fn row_for(c: &Candidate, positions: &[usize]) -> Row {
    let (dir, name) = match c.key.rsplit_once('/') {
        Some((d, n)) => (d.to_owned(), n.to_owned()),
        None => (String::new(), c.key.clone()),
    };
    let offset = c.key.chars().count() - name.chars().count();
    Row {
        target: c.target.clone(),
        name,
        name_marks: positions
            .iter()
            .filter(|&&p| p >= offset)
            .map(|p| p - offset)
            .collect(),
        dir,
    }
}

impl Source {
    pub fn capture(core: &AppCore) -> Source {
        let root = core.workspace.as_ref().map(|w| w.root.clone());
        let rel = |p: &Path| match &root {
            Some(r) => p.strip_prefix(r).map_or_else(
                |_| p.to_string_lossy().into_owned(),
                |x| x.to_string_lossy().replace('\\', "/"),
            ),
            None => p.to_string_lossy().into_owned(),
        };
        let mut initial: Vec<Candidate> = Vec::new();
        let mut seen_docs: Vec<DocumentId> = Vec::new();
        let mut push_doc = |doc: DocumentId, initial: &mut Vec<Candidate>| {
            if seen_docs.contains(&doc) {
                return;
            }
            seen_docs.push(doc);
            let Some(d) = core.workbench.document(doc) else {
                return;
            };
            let c = match &d.origin {
                DocumentOrigin::File { path, .. } => Candidate {
                    target: Target::Path(path.clone()),
                    key: rel(path),
                    boost: 20,
                },
                DocumentOrigin::Virtual { .. } => return,
                _ => Candidate {
                    target: Target::Doc(doc),
                    key: core.document_title(doc),
                    boost: 20,
                },
            };
            initial.push(c);
        };
        // Open editors: the active group's MRU first, then the other groups.
        let active = core.workbench.layout.active_group;
        let mut groups = vec![active];
        groups.extend(
            core.workbench
                .layout
                .groups_in_order()
                .into_iter()
                .filter(|g| *g != active),
        );
        for g in groups {
            let Some(group) = core.workbench.layout.group(g) else {
                continue;
            };
            let mut order = group.mru.clone();
            order.extend(group.tabs.iter().filter(|t| !group.mru.contains(t)));
            for v in order {
                if let Some(view) = core.workbench.view(v) {
                    push_doc(view.document_id, &mut initial);
                }
            }
        }
        // Remaining open documents (no view), then recent files.
        for id in core.workbench.documents.keys().copied().collect::<Vec<_>>() {
            push_doc(id, &mut initial);
        }
        for r in &core.files.recent_files {
            let p = PathBuf::from(&r.path);
            if initial.iter().any(|c| c.target == Target::Path(p.clone())) {
                continue;
            }
            initial.push(Candidate {
                target: Target::Path(p.clone()),
                key: rel(&p),
                boost: 10,
            });
        }
        Source {
            index: core
                .workspace
                .as_ref()
                .map(|w| w.index.clone())
                .unwrap_or_default(),
            indexing: core.workspace.as_ref().is_some_and(|w| !w.index_ready),
            max_files: core.settings.int("search.quickOpenMaxFiles", None).max(1) as usize,
            root,
            initial,
        }
    }

    /// Rows for `query` (without any `:line` suffix).
    pub fn rows(&self, query: &str) -> Vec<Row> {
        let q = query.trim();
        if q.is_empty() {
            return self
                .initial
                .iter()
                .take(MAX_ROWS)
                .map(|c| row_for(c, &[]))
                .collect();
        }
        let mut scored: Vec<(i32, Row)> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for c in &self.initial {
            if let Some(m) = fuzzy_match(q, &c.key) {
                if let Target::Path(p) = &c.target {
                    seen.insert(p.clone());
                }
                scored.push((m.score + c.boost, row_for(c, &m.positions)));
            }
        }
        if let Some(root) = &self.root {
            for e in self.index.iter().take(self.max_files) {
                if e.kind != NodeKind::File || e.ignored {
                    continue;
                }
                let Some(m) = fuzzy_match(q, &e.rel) else {
                    continue;
                };
                let path = root.join(&e.rel);
                if seen.contains(&path) {
                    continue;
                }
                let c = Candidate {
                    target: Target::Path(path),
                    key: e.rel.clone(),
                    boost: 0,
                };
                scored.push((m.score, row_for(&c, &m.positions)));
            }
        }
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.name.len().cmp(&b.1.name.len()))
                .then_with(|| a.1.dir.cmp(&b.1.dir))
        });
        scored.into_iter().take(MAX_ROWS).map(|(_, r)| r).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_suffixes() {
        assert_eq!(
            split_line_suffix("README.md:120"),
            ("README.md", Some((120, 1)))
        );
        assert_eq!(
            split_line_suffix("README.md:120:4"),
            ("README.md", Some((120, 4)))
        );
        assert_eq!(split_line_suffix("README.md"), ("README.md", None));
        assert_eq!(split_line_suffix("a:b"), ("a:b", None));
        assert_eq!(split_line_suffix(":12"), (":12", None));
    }

    #[test]
    fn fuzzy_ranking() {
        let entry = |rel: &str| IndexEntry {
            rel: rel.into(),
            kind: NodeKind::File,
            hidden: false,
            ignored: false,
        };
        let s = Source {
            root: Some(PathBuf::from("/w")),
            index: Arc::new(vec![
                entry("README.md"),
                entry("architecture.md"),
                entry("architecture-old.md"),
                entry("docs/architecture/design.md"),
            ]),
            max_files: 1000,
            ..Default::default()
        };
        let rows = s.rows("arch");
        let names: Vec<String> = rows
            .iter()
            .map(|r| format!("{}/{}", r.dir, r.name))
            .collect();
        assert_eq!(rows.len(), 3, "{names:?}");
        assert_eq!(rows[0].name, "architecture.md");
        assert_eq!(rows[0].name_marks, [0, 1, 2, 3]);
    }
}
