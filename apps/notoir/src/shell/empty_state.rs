//! Empty editor area (docs/specs/17 §Empty State). Actions are wired from M5.

use egui::text::LayoutJob;
use egui::{Align, FontId, Layout, RichText, TextFormat};
use notoir_core::keymap::{KeyChord, Platform};

use super::Kit;
use crate::icons::Icon;
use crate::theme::{ICON, Tok};
use crate::{fonts, t};

/// Platform-formatted chord for a binding written with `Mod`.
fn shortcut(chord: &str, platform: Platform) -> String {
    KeyChord::parse(chord, platform).map_or_else(|_| chord.to_owned(), |c| c.display(platform))
}

pub fn show(ui: &mut egui::Ui, kit: &mut Kit, platform: Platform) {
    let (primary, secondary, tertiary) = (
        kit.color(Tok::TextPrimary),
        kit.color(Tok::TextSecondary),
        kit.color(Tok::TextTertiary),
    );
    let width = 340.0_f32.min(ui.available_width());
    ui.add_space((ui.available_height() * 0.24).max(16.0));
    ui.vertical_centered(|ui| {
        let mut wordmark = LayoutJob::default();
        wordmark.append(
            "NOTOIR",
            0.0,
            TextFormat {
                font_id: fonts::extrabold(13.0),
                color: tertiary,
                extra_letter_spacing: 4.0,
                ..Default::default()
            },
        );
        ui.label(wordmark);
        ui.add_space(20.0);
        ui.label(
            RichText::new(t!("welcome:startWriting"))
                .font(fonts::semibold(24.0))
                .color(primary),
        );
        ui.add_space(28.0);

        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            let rows = [
                (Icon::Plus, "welcome:newScratchpad", "Mod+Alt+N"),
                (Icon::FolderOpen, "welcome:openFile", "Mod+O"),
                (Icon::Folder, "welcome:openWorkspace", "Mod+K Mod+O"),
            ];
            for (icon, key, chord) in rows {
                ui.horizontal(|ui| {
                    ui.set_min_height(28.0);
                    let accent = kit.color(Tok::Accent);
                    ui.add(kit.icon(ui.ctx(), icon, ICON, accent));
                    ui.add_space(4.0);
                    ui.label(RichText::new(t!(key)).size(14.0).color(primary));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(shortcut(chord, platform))
                                .font(fonts::mono(12.0))
                                .color(tertiary),
                        );
                    });
                });
            }
            ui.add_space(24.0);
            super::section_label(ui, kit, &t!("welcome:recent"));
            ui.add_space(4.0);
            ui.label(
                RichText::new(t!("welcome:noRecent"))
                    .size(13.0)
                    .color(secondary),
            );
        });

        ui.add_space(40.0);
        let mut hint = LayoutJob::default();
        hint.append(
            &shortcut("Mod+Shift+P", platform),
            0.0,
            TextFormat::simple(fonts::mono(12.0), secondary),
        );
        hint.append(
            &format!(" — {}", t!("welcome:commandPalette")),
            0.0,
            TextFormat::simple(FontId::proportional(12.0), tertiary),
        );
        ui.label(hint);
    });
}
