//! Explorer tree model (docs/specs/07): lazily loaded directory nodes, expand
//! state, sorting and the flattened row list the panel renders. Listing the
//! disk is the caller's job (`notoir-workspace`); this module only keeps what
//! was listed.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::fuzzy::fuzzy_match;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    File,
    Dir,
}

/// One directory entry as listed from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInfo {
    pub name: String,
    pub kind: NodeKind,
    /// Dotfile (or hidden attribute on Windows).
    pub hidden: bool,
    /// Matched by a `.gitignore`.
    pub ignored: bool,
    pub symlink: bool,
    /// Modification time (ms since epoch) for `explorer.sortOrder = modified`.
    pub modified: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Children {
    NotLoaded,
    Loading,
    Loaded(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
pub struct ExplorerNode {
    pub path: PathBuf,
    pub info: EntryInfo,
    pub children: Children,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    /// Folders first, then files; natural, case-insensitive.
    #[default]
    Default,
    /// Folders and files interleaved by name.
    Mixed,
    /// Newest first, folders first.
    Modified,
}

impl SortOrder {
    pub fn from_setting(s: &str) -> SortOrder {
        match s {
            "mixed" => SortOrder::Mixed,
            "modified" => SortOrder::Modified,
            _ => SortOrder::Default,
        }
    }
}

/// What the panel shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowOptions {
    pub show_hidden: bool,
    pub show_ignored: bool,
    pub sort: SortOrder,
}

/// A visible row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub kind: NodeKind,
    pub expanded: bool,
    pub loading: bool,
    pub ignored: bool,
    /// Char indices of `name` matched by the filter.
    pub highlight: Vec<usize>,
}

/// Natural, case-insensitive name order: `file2` < `file10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let lx = x.to_lowercase().next().unwrap_or(x);
                let ly = y.to_lowercase().next().unwrap_or(y);
                if lx != ly {
                    return lx.cmp(&ly);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn entry_cmp(a: &EntryInfo, b: &EntryInfo, sort: SortOrder) -> Ordering {
    let dirs_first = (b.kind == NodeKind::Dir).cmp(&(a.kind == NodeKind::Dir));
    match sort {
        SortOrder::Default => dirs_first.then_with(|| natural_cmp(&a.name, &b.name)),
        SortOrder::Mixed => natural_cmp(&a.name, &b.name),
        SortOrder::Modified => dirs_first
            .then_with(|| b.modified.cmp(&a.modified))
            .then_with(|| natural_cmp(&a.name, &b.name)),
    }
}

pub struct ExplorerTree {
    root: PathBuf,
    nodes: HashMap<PathBuf, ExplorerNode>,
    expanded: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
}

impl ExplorerTree {
    /// A tree whose root directory is expanded but not listed yet.
    pub fn new(root: PathBuf) -> Self {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let mut nodes = HashMap::new();
        nodes.insert(
            root.clone(),
            ExplorerNode {
                path: root.clone(),
                info: EntryInfo {
                    name,
                    kind: NodeKind::Dir,
                    hidden: false,
                    ignored: false,
                    symlink: false,
                    modified: 0,
                },
                children: Children::NotLoaded,
            },
        );
        let expanded = HashSet::from([root.clone()]);
        Self {
            root,
            nodes,
            expanded,
            selected: None,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn node(&self, path: &Path) -> Option<&ExplorerNode> {
        self.nodes.get(path)
    }

    pub fn is_expanded(&self, dir: &Path) -> bool {
        self.expanded.contains(dir)
    }

    /// Directories that are listed (to re-list after file changes).
    pub fn loaded_dirs(&self) -> Vec<PathBuf> {
        self.nodes
            .values()
            .filter(|n| matches!(n.children, Children::Loaded(_)))
            .map(|n| n.path.clone())
            .collect()
    }

    /// Expanded directories, for session state.
    pub fn expanded_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        v.sort();
        v
    }

    /// Marks a directory as being listed (returns false if it is unknown).
    pub fn mark_loading(&mut self, dir: &Path) -> bool {
        match self.nodes.get_mut(dir) {
            Some(n) if n.info.kind == NodeKind::Dir => {
                if n.children == Children::NotLoaded {
                    n.children = Children::Loading;
                }
                true
            }
            _ => false,
        }
    }

    /// Stores a directory listing. Subtrees of entries that still exist are
    /// kept; entries that disappeared are dropped with their subtrees.
    pub fn set_children(&mut self, dir: &Path, entries: Vec<EntryInfo>) {
        if !self.nodes.contains_key(dir) {
            return;
        }
        let new_paths: Vec<PathBuf> = entries.iter().map(|e| dir.join(&e.name)).collect();
        let keep: HashSet<&PathBuf> = new_paths.iter().collect();
        if let Some(Children::Loaded(old)) = self.nodes.get(dir).map(|n| n.children.clone()) {
            for p in old.iter().filter(|p| !keep.contains(p)) {
                self.remove_subtree(p);
            }
        }
        for (path, info) in new_paths.iter().zip(entries) {
            match self.nodes.get_mut(path) {
                Some(n) if n.info.kind == info.kind => n.info = info,
                _ => {
                    self.remove_subtree(path);
                    self.nodes.insert(
                        path.clone(),
                        ExplorerNode {
                            path: path.clone(),
                            info,
                            children: Children::NotLoaded,
                        },
                    );
                }
            }
        }
        if let Some(n) = self.nodes.get_mut(dir) {
            n.children = Children::Loaded(new_paths);
        }
    }

    /// The directory could not be listed (deleted, no permission).
    pub fn listing_failed(&mut self, dir: &Path) {
        if dir == self.root {
            self.set_children(dir, Vec::new());
            return;
        }
        self.remove_subtree(dir);
        if let Some(parent) = dir.parent()
            && let Some(Children::Loaded(list)) =
                self.nodes.get_mut(parent).map(|n| &mut n.children)
        {
            list.retain(|p| p != dir);
        }
    }

    fn remove_subtree(&mut self, path: &Path) {
        if let Some(n) = self.nodes.remove(path)
            && let Children::Loaded(children) = n.children
        {
            for c in children {
                self.remove_subtree(&c);
            }
        }
        self.expanded.remove(path);
        if self.selected.as_deref() == Some(path) {
            self.selected = None;
        }
    }

    /// Expands a directory; returns true when it must be listed first.
    pub fn expand(&mut self, dir: &Path) -> bool {
        let Some(n) = self.nodes.get(dir) else {
            return false;
        };
        if n.info.kind != NodeKind::Dir {
            return false;
        }
        self.expanded.insert(dir.to_path_buf());
        n.children == Children::NotLoaded
    }

    pub fn collapse(&mut self, dir: &Path) {
        if dir != self.root {
            self.expanded.remove(dir);
        }
    }

    /// Toggles a directory; returns true when it must be listed first.
    pub fn toggle(&mut self, dir: &Path) -> bool {
        if self.expanded.contains(dir) && dir != self.root {
            self.collapse(dir);
            false
        } else {
            self.expand(dir)
        }
    }

    pub fn collapse_all(&mut self) {
        self.expanded.retain(|p| *p == self.root);
    }

    /// Expands the ancestors of `path` (inside the root) and selects it.
    /// Returns the directories that must be listed, outermost first.
    pub fn reveal(&mut self, path: &Path) -> Vec<PathBuf> {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return Vec::new();
        };
        let mut need = Vec::new();
        let mut dir = self.root.clone();
        let comps: Vec<_> = rel.components().collect();
        for c in comps.iter().take(comps.len().saturating_sub(1)) {
            dir = dir.join(c);
            if !self.nodes.contains_key(&dir) {
                // The parent is still loading; the caller retries later.
                break;
            }
            if self.expand(&dir) {
                need.push(dir.clone());
            }
        }
        self.selected = Some(path.to_path_buf());
        need
    }

    /// Path changes from renames/moves: re-keys the subtree under `old`.
    pub fn rename(&mut self, old: &Path, new: &Path) {
        let moved: Vec<PathBuf> = self
            .nodes
            .keys()
            .filter(|p| p.starts_with(old))
            .cloned()
            .collect();
        for p in moved {
            let Some(mut n) = self.nodes.remove(&p) else {
                continue;
            };
            let np = new.join(p.strip_prefix(old).expect("prefix"));
            if p == old {
                n.info.name = new
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            if let Children::Loaded(list) = &mut n.children {
                for c in list.iter_mut() {
                    *c = new.join(c.strip_prefix(old).expect("prefix"));
                }
            }
            n.path = np.clone();
            if self.expanded.remove(&p) {
                self.expanded.insert(np.clone());
            }
            self.nodes.insert(np, n);
        }
        if let Some(sel) = &self.selected
            && let Ok(rest) = sel.strip_prefix(old)
        {
            self.selected = Some(new.join(rest));
        }
    }

    fn visible(&self, info: &EntryInfo, opts: &RowOptions) -> bool {
        (opts.show_hidden || !info.hidden) && (opts.show_ignored || !info.ignored)
    }

    /// Rows of the expanded tree (the root itself is row 0).
    pub fn rows(&self, opts: &RowOptions) -> Vec<Row> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, opts, &mut out);
        out
    }

    fn push_rows(&self, path: &Path, depth: usize, opts: &RowOptions, out: &mut Vec<Row>) {
        let Some(n) = self.nodes.get(path) else {
            return;
        };
        let expanded = self.expanded.contains(path);
        out.push(Row {
            path: path.to_path_buf(),
            name: n.info.name.clone(),
            depth,
            kind: n.info.kind,
            expanded,
            loading: expanded && n.children == Children::Loading,
            ignored: n.info.ignored,
            highlight: Vec::new(),
        });
        if !expanded {
            return;
        }
        if let Children::Loaded(children) = &n.children {
            let mut kids: Vec<&ExplorerNode> = children
                .iter()
                .filter_map(|c| self.nodes.get(c))
                .filter(|c| self.visible(&c.info, opts))
                .collect();
            kids.sort_by(|a, b| entry_cmp(&a.info, &b.info, opts.sort));
            for k in kids {
                self.push_rows(&k.path, depth + 1, opts, out);
            }
        }
    }
}

/// One entry of the background workspace index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    /// Path relative to the root, `/`-separated.
    pub rel: String,
    pub kind: NodeKind,
    pub hidden: bool,
    pub ignored: bool,
}

/// Filter mode: the matching index entries plus their ancestors, as an
/// expanded tree under `root` (row 0). Names are matched by fuzzy search.
pub fn filter_rows(
    root: &Path,
    root_name: &str,
    index: &[IndexEntry],
    query: &str,
    opts: &RowOptions,
) -> Vec<Row> {
    let mut matched: HashMap<String, Vec<usize>> = HashMap::new();
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut kinds: HashMap<String, (NodeKind, bool)> = HashMap::new();
    for e in index {
        if (!opts.show_hidden && e.hidden) || (!opts.show_ignored && e.ignored) {
            continue;
        }
        let name = e.rel.rsplit('/').next().unwrap_or(&e.rel);
        if let Some(m) = fuzzy_match(query, name) {
            let mut parent = e.rel.as_str();
            while let Some(i) = parent.rfind('/') {
                parent = &parent[..i];
                dirs.insert(parent.to_owned());
            }
            matched.insert(e.rel.clone(), m.positions);
            kinds.insert(e.rel.clone(), (e.kind, e.ignored));
        }
    }
    for d in &dirs {
        kinds.entry(d.clone()).or_insert((NodeKind::Dir, false));
    }
    // Build children lists.
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for rel in kinds.keys() {
        let parent = rel.rfind('/').map_or("", |i| &rel[..i]);
        children.entry(parent).or_default().push(rel.as_str());
    }
    let mut out = vec![Row {
        path: root.to_path_buf(),
        name: root_name.to_owned(),
        depth: 0,
        kind: NodeKind::Dir,
        expanded: true,
        loading: false,
        ignored: false,
        highlight: Vec::new(),
    }];
    #[allow(clippy::too_many_arguments)]
    fn walk(
        parent: &str,
        depth: usize,
        root: &Path,
        children: &HashMap<&str, Vec<&str>>,
        kinds: &HashMap<String, (NodeKind, bool)>,
        matched: &HashMap<String, Vec<usize>>,
        opts: &RowOptions,
        out: &mut Vec<Row>,
    ) {
        let Some(list) = children.get(parent) else {
            return;
        };
        let mut list: Vec<(&str, EntryInfo)> = list
            .iter()
            .map(|rel| {
                let (kind, ignored) = kinds[*rel];
                let name = rel.rsplit('/').next().unwrap_or(rel).to_owned();
                (
                    *rel,
                    EntryInfo {
                        name,
                        kind,
                        hidden: false,
                        ignored,
                        symlink: false,
                        modified: 0,
                    },
                )
            })
            .collect();
        list.sort_by(|a, b| entry_cmp(&a.1, &b.1, opts.sort));
        for (rel, info) in list {
            out.push(Row {
                path: root.join(rel),
                name: info.name,
                depth,
                kind: info.kind,
                expanded: info.kind == NodeKind::Dir,
                loading: false,
                ignored: info.ignored,
                highlight: matched.get(rel).cloned().unwrap_or_default(),
            });
            if info.kind == NodeKind::Dir {
                walk(rel, depth + 1, root, children, kinds, matched, opts, out);
            }
        }
    }
    walk("", 1, root, &children, &kinds, &matched, opts, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> EntryInfo {
        EntryInfo {
            name: name.into(),
            kind: NodeKind::File,
            hidden: name.starts_with('.'),
            ignored: false,
            symlink: false,
            modified: 0,
        }
    }

    fn dir(name: &str) -> EntryInfo {
        EntryInfo {
            kind: NodeKind::Dir,
            ..file(name)
        }
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth), r.name))
            .collect()
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "file1", "File2", "file10"]);
    }

    #[test]
    fn folders_first_and_hidden_filter() {
        let root = PathBuf::from("/ws");
        let mut t = ExplorerTree::new(root.clone());
        t.set_children(
            &root,
            vec![
                file("b.md"),
                dir("src"),
                file(".env"),
                file("a10.md"),
                file("a2.md"),
            ],
        );
        let opts = RowOptions::default();
        assert_eq!(
            names(&t.rows(&opts)),
            ["ws", "  src", "  a2.md", "  a10.md", "  b.md"]
        );
        let shown = RowOptions {
            show_hidden: true,
            ..opts
        };
        assert!(names(&t.rows(&shown)).contains(&"  .env".to_string()));
    }

    #[test]
    fn lazy_expand_and_relist_keeps_subtrees() {
        let root = PathBuf::from("/ws");
        let mut t = ExplorerTree::new(root.clone());
        t.set_children(&root, vec![dir("src"), file("x")]);
        let src = root.join("src");
        assert!(t.expand(&src), "needs listing");
        t.mark_loading(&src);
        assert!(t.rows(&RowOptions::default())[1].loading);
        t.set_children(&src, vec![file("main.rs")]);
        // Root re-listed after "x" was deleted: src keeps its children.
        t.set_children(&root, vec![dir("src")]);
        assert_eq!(
            names(&t.rows(&RowOptions::default())),
            ["ws", "  src", "    main.rs"]
        );
        t.collapse_all();
        assert_eq!(names(&t.rows(&RowOptions::default())), ["ws", "  src"]);
        assert!(!t.expand(&src), "already listed");
    }

    #[test]
    fn rename_rekeys_subtree() {
        let root = PathBuf::from("/ws");
        let mut t = ExplorerTree::new(root.clone());
        t.set_children(&root, vec![dir("a")]);
        t.expand(&root.join("a"));
        t.set_children(&root.join("a"), vec![file("n.md")]);
        t.selected = Some(root.join("a/n.md"));
        t.rename(&root.join("a"), &root.join("b"));
        t.set_children(&root, vec![dir("b")]);
        assert_eq!(
            names(&t.rows(&RowOptions::default())),
            ["ws", "  b", "    n.md"]
        );
        assert_eq!(t.selected, Some(root.join("b/n.md")));
    }

    #[test]
    fn reveal_expands_ancestors() {
        let root = PathBuf::from("/ws");
        let mut t = ExplorerTree::new(root.clone());
        t.set_children(&root, vec![dir("docs")]);
        let need = t.reveal(&root.join("docs/guide/x.md"));
        assert_eq!(need, vec![root.join("docs")]);
        assert_eq!(t.selected, Some(root.join("docs/guide/x.md")));
    }

    #[test]
    fn filter_keeps_ancestors() {
        let idx = vec![
            IndexEntry {
                rel: "docs".into(),
                kind: NodeKind::Dir,
                hidden: false,
                ignored: false,
            },
            IndexEntry {
                rel: "docs/readme.md".into(),
                kind: NodeKind::File,
                hidden: false,
                ignored: false,
            },
            IndexEntry {
                rel: "src/main.rs".into(),
                kind: NodeKind::File,
                hidden: false,
                ignored: false,
            },
            IndexEntry {
                rel: ".env".into(),
                kind: NodeKind::File,
                hidden: true,
                ignored: false,
            },
        ];
        let rows = filter_rows(Path::new("/ws"), "ws", &idx, "rdm", &RowOptions::default());
        assert_eq!(names(&rows), ["ws", "  docs", "    readme.md"]);
        assert_eq!(rows[2].highlight, vec![0, 3, 4]);
    }
}
