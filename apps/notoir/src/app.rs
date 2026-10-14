//! `NotoirApp`: the eframe application. In M0 it renders only the empty state
//! (docs/specs/17-shell-layout-and-panels.md §Empty State); actions are wired
//! to commands from M3 on.

use egui::text::LayoutJob;
use egui::{Align, Color32, FontId, Layout, RichText, TextFormat};

use crate::i18n::{self, Language};
use crate::icons::{Icon, IconCache};
use crate::theme::{ICON, Theme, ThemeId, Tok};
use crate::{fonts, t};

/// Env overrides used by snapshot tests and manual checks until the settings
/// system lands (M2/M3).
pub const LANG_ENV: &str = "NOTOIR_LANG";
pub const THEME_ENV: &str = "NOTOIR_THEME";

pub struct NotoirApp {
    pub language: Language,
    pub theme: Theme,
    icons: IconCache,
}

impl NotoirApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let language = Language::detect(
            notoir_platform::os_locale().as_deref(),
            std::env::var(LANG_ENV).ok().as_deref(),
        );
        let theme = std::env::var(THEME_ENV)
            .ok()
            .and_then(|id| ThemeId::from_id(&id))
            .unwrap_or_else(|| {
                let dark = cc.egui_ctx.system_theme() != Some(egui::Theme::Light);
                ThemeId::for_system(dark)
            });
        Self::with_context(&cc.egui_ctx, language, theme)
    }

    /// Builds the app for an existing context (also used by UI tests).
    pub fn with_context(ctx: &egui::Context, language: Language, theme: ThemeId) -> Self {
        let mut app = Self {
            language,
            theme: theme.load(),
            icons: IconCache::default(),
        };
        app.set_language(ctx, language);
        app.set_theme(ctx, theme);
        app
    }

    pub fn set_language(&mut self, ctx: &egui::Context, language: Language) {
        self.language = language;
        i18n::set_language(language);
        fonts::install(ctx, language);
        tracing::info!(language = language.code(), "language set");
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, theme: ThemeId) {
        self.theme = theme.load();
        self.theme.apply(ctx);
        tracing::info!(theme = theme.id(), "theme set");
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let (primary, secondary, tertiary) = (
            self.theme.color(Tok::TextPrimary),
            self.theme.color(Tok::TextSecondary),
            self.theme.color(Tok::TextTertiary),
        );
        let width = 340.0_f32.min(ui.available_width());
        let top = (ui.available_height() * 0.24).max(16.0);
        ui.add_space(top);
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

            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    let rows = [
                        (Icon::Plus, "welcome:newScratchpad", shortcut(&["Alt", "N"])),
                        (Icon::FolderOpen, "welcome:openFile", shortcut(&["O"])),
                        (Icon::Folder, "welcome:openWorkspace", chord(&["K"], &["O"])),
                    ];
                    for (icon, key, keys) in rows {
                        self.action_row(ui, icon, &t!(key), &keys);
                    }
                    ui.add_space(24.0);
                    // Section-label style (spec 21): 11 px, uppercase, tertiary.
                    ui.label(
                        RichText::new(t!("welcome:recent").to_uppercase())
                            .font(fonts::extrabold(11.0))
                            .color(tertiary),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(t!("welcome:noRecent"))
                            .size(13.0)
                            .color(secondary),
                    );
                },
            );

            ui.add_space(40.0);
            let mut hint = LayoutJob::default();
            hint.append(
                &shortcut(&["Shift", "P"]),
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

    fn action_row(&mut self, ui: &mut egui::Ui, icon: Icon, label: &str, keys: &str) {
        let color = |tok| self.theme.color(tok);
        let (accent, text, muted) = (
            color(Tok::Accent),
            color(Tok::TextPrimary),
            color(Tok::TextTertiary),
        );
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            let image = self.icons.image(ui.ctx(), icon, ICON, accent);
            ui.add(image);
            ui.add_space(4.0);
            ui.label(RichText::new(label).size(14.0).color(text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(keys).font(fonts::mono(12.0)).color(muted));
            });
        });
    }
}

/// Platform-formatted shortcut with the primary modifier (`Mod`) prepended:
/// `⌥⌘N` on macOS, `Ctrl+Alt+N` elsewhere.
pub fn shortcut(keys: &[&str]) -> String {
    format_shortcut(cfg!(target_os = "macos"), keys)
}

/// Two-stroke chord, e.g. `⌘K ⌘O` / `Ctrl+K Ctrl+O`.
pub fn chord(first: &[&str], second: &[&str]) -> String {
    format!("{} {}", shortcut(first), shortcut(second))
}

pub fn format_shortcut(mac: bool, keys: &[&str]) -> String {
    if mac {
        // macOS order: ⌃ ⌥ ⇧ ⌘ then the key.
        let mut out = String::new();
        for (name, symbol) in [("Ctrl", "⌃"), ("Alt", "⌥"), ("Shift", "⇧")] {
            if keys.contains(&name) {
                out.push_str(symbol);
            }
        }
        out.push('⌘');
        for key in keys {
            if !matches!(*key, "Ctrl" | "Alt" | "Shift") {
                out.push_str(key);
            }
        }
        out
    } else {
        let mut parts = vec!["Ctrl"];
        parts.extend(keys.iter().copied());
        parts.join("+")
    }
}

impl eframe::App for NotoirApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let frame = egui::Frame::new().fill(self.theme.color(Tok::BgEditor));
        egui::CentralPanel::no_frame()
            .frame(frame)
            .show(ui, |ui| self.empty_state(ui));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c: Color32 = self.theme.color(Tok::BgApp);
        egui::Rgba::from(c).to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_follow_platform_conventions() {
        assert_eq!(format_shortcut(true, &["Alt", "N"]), "⌥⌘N");
        assert_eq!(format_shortcut(true, &["Shift", "P"]), "⇧⌘P");
        assert_eq!(format_shortcut(false, &["Alt", "N"]), "Ctrl+Alt+N");
        assert_eq!(format_shortcut(false, &["Shift", "P"]), "Ctrl+Shift+P");
    }
}
