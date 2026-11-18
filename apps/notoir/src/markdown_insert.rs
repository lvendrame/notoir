//! Markdown insertion state, command helpers and context-menu model.
use std::path::{Component, Path};

use notoir_core::explorer::NodeKind;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_editor::{Selection, markdown_edit};
use serde_json::json;

use crate::core::{AppCore, Effect};
use crate::menus::MenuItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertDialogKind {
    Link,
    Image,
    Code,
    Table,
}

#[derive(Clone, Debug)]
pub struct InsertDialog {
    pub view: ViewId,
    pub document: DocumentId,
    pub revision: u64,
    pub selection: Selection,
    pub kind: InsertDialogKind,
    pub columns: usize,
    pub rows: usize,
    pub caption: String,
    pub destination: String,
    pub selected: usize,
    pub fresh: bool,
    pub error: bool,
}

impl AppCore {
    pub fn open_md_insert_dialog(&mut self, view: ViewId, kind: InsertDialogKind) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(d) = self.workbench.document(v.document_id) else {
            return;
        };
        if !self.is_markdown(d.id) || d.is_read_only() {
            return;
        }
        let r = v.selection.primary();
        self.markdown.insert_dialog = Some(InsertDialog {
            view,
            document: d.id,
            revision: d.revision,
            selection: v.selection.clone(),
            kind,
            columns: 2,
            rows: 1,
            caption: d.buffer.rope().slice(r.from()..r.to()).to_string(),
            destination: String::new(),
            selected: 0,
            fresh: true,
            error: false,
        });
    }

    pub fn md_insert_target_valid(&self, dialog: &InsertDialog) -> bool {
        self.workbench
            .view(dialog.view)
            .is_some_and(|v| v.document_id == dialog.document)
            && self.workbench.document(dialog.document).is_some_and(|d| {
                d.revision == dialog.revision && !d.is_read_only() && self.is_markdown(d.id)
            })
    }

    pub fn confirm_md_insert(&mut self, language: Option<&str>) {
        let Some(dialog) = self.markdown.insert_dialog.clone() else {
            return;
        };
        if !self.md_insert_target_valid(&dialog) {
            if let Some(d) = &mut self.markdown.insert_dialog {
                d.error = true;
            }
            return;
        }
        let edit = if dialog.kind == InsertDialogKind::Table {
            let d = self.workbench.document(dialog.document).expect("validated");
            markdown_edit::insert(
                d.buffer.rope(),
                &dialog.selection,
                markdown_edit::Insert::Table {
                    columns: dialog.columns,
                    rows: dialog.rows,
                },
            )
        } else if dialog.kind == InsertDialogKind::Code {
            let Some(language) = language
                .filter(|l| l.is_empty() || notoir_editor::LanguageId::from_id(l).is_some())
            else {
                return;
            };
            let d = self.workbench.document(dialog.document).expect("validated");
            markdown_edit::insert(
                d.buffer.rope(),
                &dialog.selection,
                markdown_edit::Insert::Code(if language == "shellscript" {
                    "sh"
                } else {
                    language
                }),
            )
        } else {
            let url = dialog.destination.trim();
            if url.is_empty() || url.starts_with('@') {
                return;
            }
            let caption = if dialog.caption.is_empty() && dialog.kind == InsertDialogKind::Link {
                url
            } else {
                &dialog.caption
            };
            Some(markdown_edit::link(
                dialog.selection.primary(),
                caption,
                url,
                dialog.kind == InsertDialogKind::Image,
            ))
        };
        if let Some(edit) = edit {
            if self
                .workbench
                .edit(
                    dialog.view,
                    edit.changes,
                    edit.selection,
                    edit.kind,
                    self.now_ms,
                )
                .is_ok()
            {
                self.markdown.insert_dialog = None;
                self.effects.push(Effect::FocusEditor(dialog.view));
            } else if let Some(d) = &mut self.markdown.insert_dialog {
                d.error = true;
            }
        }
    }

    pub fn insert_md_block(&mut self, view: ViewId, kind: markdown_edit::Insert<'_>) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(d) = self.workbench.document(v.document_id) else {
            return;
        };
        if d.is_read_only() || !self.is_markdown(d.id) {
            return;
        }
        if let Some(edit) = markdown_edit::insert(d.buffer.rope(), &v.selection, kind) {
            let _ = self
                .workbench
                .edit(view, edit.changes, edit.selection, edit.kind, self.now_ms);
            self.effects.push(Effect::FocusEditor(view));
        }
    }

    /// Workspace-only results, ranked by filename relevance before fuzzy score.
    /// The explicit kind also works while the UI temporarily owns the draft.
    pub fn md_file_candidates(&self, query: &str, images_only: bool) -> Vec<String> {
        let Some(ws) = &self.workspace else {
            return Vec::new();
        };
        file_candidates(&ws.index, query, images_only)
    }

    pub fn choose_md_file(&mut self, relative: &str) {
        let Some(ws) = &self.workspace else {
            return;
        };
        if !ws
            .index
            .iter()
            .any(|e| e.rel == relative && e.kind == NodeKind::File && !e.ignored)
        {
            return;
        }
        let Some(dialog) = &self.markdown.insert_dialog else {
            return;
        };
        if dialog.kind == InsertDialogKind::Image && !is_image_file(relative) {
            return;
        }
        let target = ws.root.join(relative);
        let base = self
            .workbench
            .document(dialog.document)
            .and_then(|d| d.path())
            .and_then(Path::parent)
            .unwrap_or(&ws.root);
        let path = relative_path(base, &target);
        let path = if path.starts_with('@')
            || (!Path::new(&path).is_absolute()
                && path.split('/').next().is_some_and(|p| p.contains(':')))
        {
            format!("./{path}")
        } else {
            path
        };
        // File names containing URL fragment/query delimiters must stay literal.
        let path = path
            .replace('%', "%25")
            .replace('#', "%23")
            .replace('?', "%3F");
        if let Some(dialog) = &mut self.markdown.insert_dialog {
            dialog.destination = path;
            if dialog.caption.is_empty() && dialog.kind == InsertDialogKind::Link {
                dialog.caption = Path::new(relative)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
            }
            dialog.selected = 0;
        }
    }
}

fn is_image_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            ["png", "jpg", "jpeg", "gif", "webp", "svg"]
                .iter()
                .any(|supported| e.eq_ignore_ascii_case(supported))
        })
}

fn file_candidates(
    index: &[notoir_core::explorer::IndexEntry],
    query: &str,
    images_only: bool,
) -> Vec<String> {
    let query = query.trim().replace('\\', "/").to_lowercase();
    let mut rows: Vec<_> = index
        .iter()
        .filter(|e| {
            e.kind == NodeKind::File && !e.ignored && (!images_only || is_image_file(&e.rel))
        })
        .filter_map(|e| {
            let path = e.rel.to_lowercase();
            let name = path.rsplit('/').next().unwrap_or(&path);
            let class = if query.is_empty() {
                7
            } else if path == query || name == query {
                0
            } else if Path::new(name).file_stem().and_then(|s| s.to_str()) == Some(query.as_str()) {
                1
            } else if name.starts_with(&query) {
                2
            } else if name.contains(&query) {
                3
            } else if path.contains(&query) {
                4
            } else if notoir_core::fuzzy::fuzzy_match(&query, name).is_some() {
                5
            } else {
                6
            };
            // Literal matches must survive even when the fuzzy matcher's bounded
            // start-position search cannot align a long, repetitive filename.
            let score = match notoir_core::fuzzy::fuzzy_match(&query, &e.rel) {
                Some(m) => m.score,
                None if class <= 4 => 0,
                None => return None,
            };
            Some((class, score, e.rel.clone()))
        })
        .collect();
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.len().cmp(&b.2.len()))
            .then(a.2.cmp(&b.2))
    });
    rows.into_iter()
        .take(200)
        .map(|(_, _, path)| path)
        .collect()
}

fn relative_path(base: &Path, target: &Path) -> String {
    let a: Vec<_> = base.components().collect();
    let b: Vec<_> = target.components().collect();
    let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return target.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = a[common..]
        .iter()
        .filter(|c| matches!(c, Component::Normal(_)))
        .map(|_| "..".into())
        .collect();
    parts.extend(
        b[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

pub fn context_items(view: ViewId) -> MenuItem {
    fn command(view: ViewId, id: &'static str, key: &str, mut args: serde_json::Value) -> MenuItem {
        args["view"] = json!(view.0);
        MenuItem::Command {
            id,
            args,
            label: Some(format!("markdown:insert.{key}")),
            checked: None,
        }
    }
    let block = |kind: &str, label: &str| {
        command(view, "markdown.insertBlock", label, json!({"kind":kind}))
    };
    let sub = |label: &'static str, items| MenuItem::Submenu { label, items };
    let heading = (1..=6)
        .map(|level| {
            command(
                view,
                "markdown.insertBlock",
                &format!("h{level}"),
                json!({"kind":"heading", "level":level}),
            )
        })
        .collect();
    let mut code: Vec<_> = [
        ("", "general"),
        ("javascript", "javascript"),
        ("typescript", "typescript"),
        ("shellscript", "shell"),
        ("python", "python"),
        ("rust", "rust"),
        ("json", "json"),
        ("html", "html"),
        ("css", "css"),
        ("sql", "sql"),
        ("yaml", "yaml"),
    ]
    .into_iter()
    .map(|(language, label)| {
        command(
            view,
            "markdown.insertBlock",
            label,
            json!({"kind":"code", "language":language}),
        )
    })
    .collect();
    code.push(command(
        view,
        "markdown.insertCodePicker",
        "more",
        json!({}),
    ));
    sub(
        "markdown:insert.menu",
        vec![
            command(view, "markdown.insertLink", "link", json!({})),
            sub("markdown:insert.heading", heading),
            sub("markdown:insert.code", code),
            block("quote", "quote"),
            sub(
                "markdown:insert.formatting",
                vec![
                    command(view, "markdown.toggleBold", "bold", json!({})),
                    command(view, "markdown.toggleItalic", "italic", json!({})),
                    command(
                        view,
                        "markdown.toggleStrikethrough",
                        "strikethrough",
                        json!({}),
                    ),
                    command(view, "markdown.toggleCode", "inlineCode", json!({})),
                ],
            ),
            sub(
                "markdown:insert.list",
                vec![
                    block("bullet", "bullet"),
                    block("numbered", "numbered"),
                    block("task", "task"),
                ],
            ),
            command(view, "markdown.insertImage", "image", json!({})),
            block("table", "table"),
            block("rule", "rule"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(rel: &str) -> notoir_core::explorer::IndexEntry {
        notoir_core::explorer::IndexEntry {
            rel: rel.into(),
            kind: NodeKind::File,
            hidden: false,
            ignored: false,
        }
    }

    #[test]
    fn exact_filename_matches_survive_the_result_limit_and_rank_first() {
        let mut index: Vec<_> = (0..300)
            .map(|i| file(&format!("notes/{i}/application_results.rs")))
            .collect();
        index.extend([
            file("apps/notoir/src/app.rs"),
            file("src/app.rs.old"),
            file("app.rs/notes.md"),
        ]);
        let rows = file_candidates(&index, "app.rs", false);
        assert_eq!(rows.len(), 200);
        assert_eq!(rows[0], "apps/notoir/src/app.rs");
        assert!(
            rows.iter().position(|p| p == "src/app.rs.old").unwrap()
                < rows.iter().position(|p| p == "app.rs/notes.md").unwrap()
        );
        assert_eq!(
            file_candidates(&index, "APPS/NOTOIR/SRC/APP.RS", false),
            ["apps/notoir/src/app.rs"]
        );
    }

    #[test]
    fn exact_stems_and_literal_repetitive_names_beat_partial_matches() {
        let name = format!("{}.rs", "a".repeat(40));
        let index = [
            file("application.rs"),
            file("apps/notoir/src/app.rs"),
            file(&name),
        ];
        assert_eq!(
            file_candidates(&index, "app", false)[0],
            "apps/notoir/src/app.rs"
        );
        assert_eq!(file_candidates(&index, &name, false), [name]);
    }

    #[test]
    fn image_candidates_filter_extensions_before_ranking_and_limiting() {
        let mut index: Vec<_> = (0..250).map(|i| file(&format!("logo-{i}.rs"))).collect();
        for path in [
            "assets/logo.png",
            "assets/logo.JPG",
            "assets/logo.jpeg",
            "assets/logo.gif",
            "assets/logo.webp",
            "assets/logo.svg",
            "assets/logo.png.rs",
        ] {
            index.push(file(path));
        }
        let rows = file_candidates(&index, "logo", true);
        assert_eq!(rows.len(), 6);
        assert!(rows.iter().all(|path| is_image_file(path)));
        assert_eq!(
            file_candidates(&index, "logo.png", true),
            ["assets/logo.png"]
        );
        let mut ignored = file("ignored/logo.png");
        ignored.ignored = true;
        let mut folder = file("folder.png");
        folder.kind = NodeKind::Dir;
        assert!(file_candidates(&[ignored, folder], "", true).is_empty());
    }

    #[test]
    fn relative_links() {
        assert_eq!(
            relative_path(Path::new("/ws/docs"), Path::new("/ws/notes/a.md")),
            "../notes/a.md"
        );
        assert_eq!(
            relative_path(Path::new("/ws"), Path::new("/ws/a.md")),
            "a.md"
        );
    }
}
