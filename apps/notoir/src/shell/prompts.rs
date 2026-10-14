//! Confirmation prompts for unsaved changes, Scratchpads, save conflicts,
//! revert and binary files (docs/specs/08, 09). One at a time, modal.

use egui::{Align, Key, Layout, RichText};

use super::Kit;
use crate::core::AppCore;
use crate::files::{Answer, Prompt};
use crate::t;
use crate::theme::Tok;

struct Spec {
    title: String,
    body: String,
    list: Vec<String>,
    /// (label, answer, danger)
    buttons: Vec<(String, Answer, bool)>,
}

fn spec(core: &AppCore, prompt: &Prompt) -> Spec {
    let title_of = |d| core.document_title(d);
    match prompt {
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
            title: t!(
                "dialogs:binary.title",
                name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            body: t!("dialogs:binary.body"),
            list: Vec::new(),
            buttons: vec![
                (t!("dialogs:binary.openAnyway"), Answer::Primary, false),
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
        ui.label(
            RichText::new(&spec.body)
                .size(13.0)
                .color(kit.color(Tok::TextSecondary)),
        );
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
                if ui.button(text).clicked() {
                    answer = Some(*a);
                }
            }
        });
    });
    answer
}
