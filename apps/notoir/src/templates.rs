//! File templates (docs/specs/34 §Templates): `.notoir/templates/*.md` in
//! the workspace (shared) and `<config>/templates/` (personal). "New File
//! from Template…" picks one, names the file inline in the Explorer (or
//! opens an untitled document without a workspace) and fills in `{{date}}`,
//! `{{time}}`, `{{title}}` and `{{cursor}}`. The new file is created empty
//! and its content goes through the editor and the save pipeline.

use std::path::{Path, PathBuf};

use notoir_core::document::DocumentOrigin;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::workbench::TabPosition;
use notoir_editor::{EditKind, Selection};

use crate::core::{AppCore, Effect};
use crate::io::IoResult;
use crate::workspace::EditKind as ExplorerEdit;

/// The workspace's template folder (relative to the root).
pub const WORKSPACE_DIR: &str = ".notoir/templates";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub path: PathBuf,
    /// From the workspace (shared) rather than the personal folder.
    pub shared: bool,
}

/// A template chosen for a new file, waiting for its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateText {
    pub name: String,
    /// The template's extension, added when the typed name has none.
    pub ext: String,
    pub text: String,
}

/// Template text to put into a newly created file once it opens.
#[derive(Clone, Debug)]
struct PendingFill {
    path: PathBuf,
    text: String,
    cursor: Option<usize>,
}

#[derive(Default)]
pub struct TemplatesState {
    /// The picker's rows.
    pub list: Vec<Template>,
    /// Explorer folder the new file goes into.
    dir: Option<PathBuf>,
    pending: Option<PendingFill>,
}

/// Values substituted into a template.
pub struct Vars<'a> {
    pub date: &'a str,
    pub time: &'a str,
    pub title: &'a str,
}

/// Replaces the variables; returns the text and the char offset of the
/// first `{{cursor}}` (later ones are dropped). Unknown `{{…}}` stay as
/// written.
pub fn expand(template: &str, vars: &Vars) -> (String, Option<usize>) {
    let mut out = String::with_capacity(template.len());
    let mut cursor = None;
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = after[..end].trim();
        match name {
            "date" => out.push_str(vars.date),
            "time" => out.push_str(vars.time),
            "title" => out.push_str(vars.title),
            "cursor" => {
                cursor.get_or_insert(out.chars().count());
            }
            _ => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    (out, cursor)
}

/// Templates in the workspace folder (`*.md`), then the personal folder
/// (any file), each sorted by name.
pub fn list(root: Option<&Path>, config: &Path) -> Vec<Template> {
    let mut out = Vec::new();
    let dirs = root
        .map(|r| (r.join(WORKSPACE_DIR), true))
        .into_iter()
        .chain([(config.join("templates"), false)]);
    for (dir, shared) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found: Vec<Template> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                !name.starts_with('.')
                    && (!shared || p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")))
            })
            .map(|path| Template {
                name: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path,
                shared,
            })
            .collect();
        found.sort_by_key(|t| t.name.to_lowercase());
        out.extend(found);
    }
    out
}

fn now_vars() -> (String, String) {
    let now = chrono::Local::now();
    (
        now.format("%Y-%m-%d").to_string(),
        now.format("%H:%M").to_string(),
    )
}

impl AppCore {
    /// File → New File from Template…: lists the templates in the
    /// background, then shows the picker.
    pub fn templates_start(&mut self, dir: Option<PathBuf>) {
        self.templates.dir = dir;
        let root = self.workspace.as_ref().map(|w| w.root.clone());
        let config = self.paths.config.clone();
        self.fs
            .submit(move || Some(IoResult::TemplatesListed(list(root.as_deref(), &config))));
    }

    pub fn templates_listed(&mut self, list: Vec<Template>) {
        if list.is_empty() {
            let personal = self.paths.config.join("templates");
            self.notify(Notification::new(
                Severity::Info,
                Translatable::new("notifications:noTemplates")
                    .arg("shared", WORKSPACE_DIR)
                    .arg("personal", personal.display().to_string()),
            ));
            return;
        }
        self.templates.list = list;
        self.effects.push(Effect::OpenPicker(
            crate::shell::overlays::PickerKind::Templates,
        ));
    }

    /// A template was picked: read it in the background.
    pub fn templates_pick(&mut self, path: &str) {
        let Some(t) = self
            .templates
            .list
            .iter()
            .find(|t| t.path.to_string_lossy() == path)
            .cloned()
        else {
            return;
        };
        self.fs.submit(move || {
            let result = std::fs::read(&t.path).map(|b| {
                let d = notoir_editor::encoding::decode(&b, None);
                notoir_editor::line_ending::normalize(&d.text)
            });
            Some(IoResult::TemplateLoaded(Box::new((t, result))))
        });
    }

    pub fn templates_loaded(&mut self, t: Template, text: std::io::Result<String>) {
        let text = match text {
            Ok(text) => text,
            Err(e) => {
                self.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("notifications:templateUnreadable")
                            .arg("name", t.name.clone()),
                    )
                    .detail(e.to_string()),
                );
                return;
            }
        };
        let ext = t
            .path
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let template = TemplateText {
            name: t.name,
            ext,
            text,
        };
        match self
            .templates
            .dir
            .take()
            .filter(|_| self.workspace.is_some())
        {
            Some(dir) => {
                self.exit_maximize();
                self.panels.show(notoir_core::panels::PanelId::Explorer);
                self.begin_edit(ExplorerEdit::NewFile {
                    dir,
                    template: Some(template),
                });
            }
            None => self.templates_untitled(&template),
        }
    }

    /// Without a workspace: an untitled document with the template.
    fn templates_untitled(&mut self, t: &TemplateText) {
        let (date, time) = now_vars();
        let (text, cursor) = expand(
            &t.text,
            &Vars {
                date: &date,
                time: &time,
                title: &t.name,
            },
        );
        let language = notoir_editor::language::detect(
            Some(&format!("untitled.{}", t.ext)),
            text.lines().next().unwrap_or_default(),
            &[],
        );
        let n = self.workbench.next_untitled_number();
        let doc = self.workbench.open_document(
            DocumentOrigin::Untitled { n },
            notoir_core::document::LoadedText::empty(language),
        );
        let group = self.workbench.layout.active_group;
        if let Ok(view) = self
            .workbench
            .open_view(doc, group, TabPosition::AfterActive)
        {
            self.templates_insert(view, &text, cursor);
            self.effects.push(Effect::FocusEditor(view));
        }
    }

    /// Explorer New File with a template: the name was accepted; the file is
    /// created empty and filled once it opens.
    pub(crate) fn templates_arm(&mut self, target: &Path, t: &TemplateText) {
        let (date, time) = now_vars();
        let title = target
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (text, cursor) = expand(
            &t.text,
            &Vars {
                date: &date,
                time: &time,
                title: &title,
            },
        );
        self.templates.pending = Some(PendingFill {
            path: target.to_path_buf(),
            text,
            cursor,
        });
    }

    /// A file finished opening: fill it if it is the new templated file.
    pub(crate) fn templates_opened(&mut self, doc: DocumentId, view: ViewId, path: &Path) {
        if self
            .templates
            .pending
            .as_ref()
            .is_none_or(|p| p.path != path)
        {
            return;
        }
        let p = self.templates.pending.take().expect("checked");
        self.templates_insert(view, &p.text, p.cursor);
        // Written like any edit: through the save pipeline.
        self.save(doc);
    }

    fn templates_insert(&mut self, view: ViewId, text: &str, cursor: Option<usize>) {
        if text.is_empty() {
            return;
        }
        let sel = Selection::caret(0);
        let Some(res) = notoir_editor::edit_ops::insert(&sel, text, EditKind::Insert) else {
            return;
        };
        let now = self.now_ms;
        let at = cursor.unwrap_or(0);
        let _ = self
            .workbench
            .edit(view, res.changes, Selection::caret(at), res.kind, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_variables_and_finds_the_cursor() {
        let vars = Vars {
            date: "2026-10-16",
            time: "09:05",
            title: "Weekly Review",
        };
        let (text, cursor) = expand(
            "# {{title}}\n{{ date }} {{time}}\n\n- {{cursor}}\n{{cursor}}{{unknown}} {{open",
            &vars,
        );
        assert_eq!(
            text,
            "# Weekly Review\n2026-10-16 09:05\n\n- \n{{unknown}} {{open"
        );
        assert_eq!(
            cursor,
            Some("# Weekly Review\n2026-10-16 09:05\n\n- ".chars().count())
        );
        assert_eq!(expand("plain", &vars), ("plain".into(), None));
        // Offsets are in chars.
        assert_eq!(expand("é{{cursor}}", &vars).1, Some(1));
    }

    #[test]
    fn lists_shared_then_personal_templates() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("ws");
        let config = d.path().join("config");
        let shared = root.join(WORKSPACE_DIR);
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::create_dir_all(config.join("templates")).unwrap();
        for (dir, name) in [
            (&shared, "Meeting.md"),
            (&shared, "daily.md"),
            (&shared, "notes.txt"),
            (&shared, ".hidden.md"),
        ] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        std::fs::write(config.join("templates/Letter.txt"), "x").unwrap();
        let names: Vec<(String, bool)> = list(Some(&root), &config)
            .into_iter()
            .map(|t| (t.name, t.shared))
            .collect();
        assert_eq!(
            names,
            [
                ("daily".into(), true),
                ("Meeting".into(), true),
                ("Letter".into(), false)
            ]
        );
        assert_eq!(list(None, &config).len(), 1);
    }
}
