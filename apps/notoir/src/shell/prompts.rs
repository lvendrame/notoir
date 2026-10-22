//! Confirmation prompts for unsaved changes, Scratchpads, save conflicts,
//! revert and binary files (docs/specs/08, 09). One at a time, modal.

use egui::{Align, Key, Layout, RichText};

use super::Kit;
use crate::core::AppCore;
use crate::files::{Answer, Prompt};
use crate::git_write::GitPrompt;
use crate::t;
use crate::theme::Tok;

struct Spec {
    title: String,
    body: String,
    list: Vec<String>,
    /// (label, answer, danger)
    buttons: Vec<(String, Answer, bool)>,
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Source Control confirmations (spec 28).
fn git_spec(p: &GitPrompt) -> Spec {
    let base = |rel: &str| rel.rsplit('/').next().unwrap_or(rel).to_owned();
    match p {
        GitPrompt::Discard { tracked, untracked } => {
            let all: Vec<&String> = tracked.iter().chain(untracked).collect();
            let (title, confirm) = if all.len() == 1 {
                let name = base(all[0]);
                let title = if untracked.is_empty() {
                    t!("git:discard.title", name = name)
                } else {
                    t!("git:discard.titleUntracked", name = name)
                };
                (title, t!("git:discard.confirm"))
            } else {
                (
                    t!("git:discard.titleMany", count = all.len()),
                    t!("git:discard.confirmMany", count = all.len()),
                )
            };
            let mut body = t!("git:discard.body");
            if !untracked.is_empty() {
                body.push(' ');
                body.push_str(&t!("git:discard.trash"));
            }
            Spec {
                title,
                body,
                list: if all.len() > 1 {
                    all.iter().map(|r| r.to_string()).collect()
                } else {
                    Vec::new()
                },
                buttons: vec![
                    (confirm, Answer::Primary, true),
                    (t!("common:cancel"), Answer::Cancel, false),
                ],
            }
        }
        GitPrompt::SmartCommit { .. } => Spec {
            title: t!("git:smartCommit.title"),
            body: t!("git:smartCommit.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("git:smartCommit.confirm"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        GitPrompt::Diverged { upstream } => Spec {
            title: t!("git:diverged.title", upstream = upstream),
            body: String::new(),
            list: Vec::new(),
            buttons: vec![
                (t!("git:diverged.rebase"), Answer::Primary, false),
                (t!("git:diverged.merge"), Answer::Secondary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        GitPrompt::Publish { branch, remote }
        | GitPrompt::AutoPublish {
            target: crate::git_auto::PushTarget { branch, .. },
            remote,
        } => Spec {
            title: t!("git:publish.title", branch = branch, remote = remote),
            body: t!("git:publish.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("git:publish.confirm"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        GitPrompt::SyncConfirm => Spec {
            title: t!("git:syncConfirm.title"),
            body: t!("git:syncConfirm.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("common:ok"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        GitPrompt::Init(path) => Spec {
            title: t!("git:init.title", name = file_name(path)),
            body: t!("git:init.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("git:init.confirm"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        GitPrompt::OpenCloned(path) => Spec {
            title: t!("git:clone.openTitle", name = file_name(path)),
            body: String::new(),
            list: Vec::new(),
            buttons: vec![
                (t!("git:clone.open"), Answer::Primary, false),
                (t!("dialogs:workspace.newWindow"), Answer::Secondary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
    }
}

fn spec(core: &AppCore, prompt: &Prompt) -> Spec {
    let title_of = |d| core.document_title(d);
    match prompt {
        Prompt::Git(p) => git_spec(p),
        Prompt::Notes(crate::notes::NotesPrompt::Create { name, .. }) => Spec {
            title: t!("notes:create.title", name = name.as_str()),
            body: t!("notes:create.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("notes:create.confirm"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::Notes(crate::notes::NotesPrompt::UpdateLinks {
            old_stem,
            new_stem,
            files,
        }) => Spec {
            title: t!(
                "notes:rename.title",
                count = files.len(),
                old = old_stem.as_str(),
                new = new_stem.as_str()
            ),
            body: t!("notes:rename.body"),
            list: files.clone(),
            buttons: vec![
                (t!("notes:rename.confirm"), Answer::Primary, false),
                (t!("notes:rename.skip"), Answer::Cancel, false),
            ],
        },
        Prompt::Unsaved { docs, .. } if docs.len() == 1 => Spec {
            title: t!("dialogs:unsaved.title", name = title_of(docs[0])),
            body: t!("dialogs:unsaved.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("common:save"), Answer::Primary, false),
                (t!("common:dontSave"), Answer::Secondary, true),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::Unsaved { docs, .. } => Spec {
            title: t!("dialogs:unsaved.titleMany", count = docs.len()),
            body: t!("dialogs:unsaved.body"),
            list: docs.iter().map(|d| title_of(*d)).collect(),
            buttons: vec![
                (t!("dialogs:unsaved.saveAll"), Answer::Primary, false),
                (t!("common:dontSave"), Answer::Secondary, true),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::ScratchpadClose { doc, .. } => Spec {
            title: t!("dialogs:scratchpad.title", name = title_of(*doc)),
            body: t!("dialogs:scratchpad.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:scratchpad.keep"), Answer::Primary, false),
                (t!("dialogs:scratchpad.saveAs"), Answer::SaveAs, false),
                (t!("dialogs:scratchpad.delete"), Answer::Secondary, true),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::SaveConflict { doc } => Spec {
            title: t!("dialogs:conflict.title", name = title_of(*doc)),
            body: t!("dialogs:conflict.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:conflict.overwrite"), Answer::Primary, true),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::Revert { doc } => Spec {
            title: t!("dialogs:revert.title", name = title_of(*doc)),
            body: t!("dialogs:revert.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:revert.confirm"), Answer::Primary, true),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::Binary { path, .. } => Spec {
            title: t!("dialogs:binary.title", name = file_name(path)),
            body: t!("dialogs:binary.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:binary.openAnyway"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::WorkspaceReplace { path } => {
            let this = (t!("dialogs:workspace.thisWindow"), Answer::Primary, false);
            let new = (t!("dialogs:workspace.newWindow"), Answer::Secondary, false);
            // Unsaved work in this window: the safe default is a new window.
            let dirty = core.workbench.documents.values().any(|d| d.is_dirty());
            let mut buttons = if dirty {
                vec![new, this]
            } else {
                vec![this, new]
            };
            buttons.push((t!("common:cancel"), Answer::Cancel, false));
            Spec {
                title: t!("dialogs:workspace.title", name = file_name(path)),
                body: t!("dialogs:workspace.body"),
                list: Vec::new(),
                buttons,
            }
        }
        Prompt::Delete { paths, permanent } => {
            let name = if paths.len() == 1 {
                file_name(&paths[0])
            } else {
                String::new()
            };
            let (title, body, confirm) = match (paths.len() == 1, permanent) {
                (true, false) => (
                    t!("dialogs:delete.title", name = name),
                    t!("dialogs:delete.body"),
                    t!("dialogs:delete.confirm"),
                ),
                (false, false) => (
                    t!("dialogs:delete.titleMany", count = paths.len()),
                    t!("dialogs:delete.body"),
                    t!("dialogs:delete.confirm"),
                ),
                (true, true) => (
                    t!("dialogs:delete.permanentTitle", name = name),
                    t!("dialogs:delete.permanentBody"),
                    t!("dialogs:delete.permanentConfirm"),
                ),
                (false, true) => (
                    t!("dialogs:delete.permanentTitleMany", count = paths.len()),
                    t!("dialogs:delete.permanentBody"),
                    t!("dialogs:delete.permanentConfirm"),
                ),
            };
            Spec {
                title,
                body,
                list: if paths.len() > 1 {
                    paths.iter().map(|p| file_name(p)).collect()
                } else {
                    Vec::new()
                },
                buttons: vec![
                    (confirm, Answer::Primary, true),
                    (t!("common:cancel"), Answer::Cancel, false),
                ],
            }
        }
        Prompt::DiscardRecovery { count } => Spec {
            title: t!("dialogs:recovery.discardTitle", count = count),
            body: t!("dialogs:recovery.discardBody"),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:recovery.discard"), Answer::Primary, true),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::ReplaceAll { count, files } => Spec {
            title: t!("dialogs:replaceAll.title", count = *count, files = *files),
            body: String::new(),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:replaceAll.replace"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::OpenLink { url } => Spec {
            title: t!("dialogs:openLink.title"),
            body: url.clone(),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:openLink.open"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
        Prompt::Move { srcs, dir } => Spec {
            title: if srcs.len() == 1 {
                t!(
                    "dialogs:move.title",
                    name = file_name(&srcs[0]),
                    folder = file_name(dir)
                )
            } else {
                t!(
                    "dialogs:move.titleMany",
                    count = srcs.len(),
                    folder = file_name(dir)
                )
            },
            body: String::new(),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:move.confirm"), Answer::Primary, false),
                (t!("common:cancel"), Answer::Cancel, false),
            ],
        },
    }
}

/// Shows the front prompt; returns the user's answer.
pub fn show(ctx: &egui::Context, kit: &mut Kit, core: &AppCore) -> Option<Answer> {
    let prompt = core.files.prompts.front()?;
    let spec = spec(core, prompt);
    let mut answer = None;
    let (enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
            i.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if enter {
        return Some(spec.buttons[0].1);
    }
    if esc {
        return Some(Answer::Cancel);
    }
    egui::Modal::new(egui::Id::new("prompt")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(
            RichText::new(&spec.title)
                .font(crate::fonts::semibold(15.0))
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.add_space(6.0);
        if !spec.body.is_empty() {
            ui.label(
                RichText::new(&spec.body)
                    .size(13.0)
                    .color(kit.color(Tok::TextSecondary)),
            );
        }
        for item in &spec.list {
            ui.label(
                RichText::new(format!("• {item}"))
                    .size(13.0)
                    .color(kit.color(Tok::TextPrimary)),
            );
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for (label, a, danger) in spec.buttons.iter().rev() {
                let text = if *danger {
                    RichText::new(label).color(kit.color(Tok::Danger))
                } else {
                    RichText::new(label)
                };
                let response = ui.button(text);
                crate::live::track(&response, &format!("prompt.{a:?}"));
                if response.clicked() {
                    answer = Some(*a);
                }
            }
        });
    });
    answer
}
