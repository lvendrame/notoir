//! `NotoirApp`: the eframe application and its frame loop.
//!
//! Per frame: pump background results → keyboard → render chrome (collecting
//! command invocations) → run queued commands → apply effects that need the
//! egui context → drain events.

use std::time::Instant;

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::panels::PanelArea;
use notoir_platform::system_fonts::{self, Script};
use serde_json::json;

use crate::core::{AppCore, Effect, StartOptions, ThemeChoice};
use crate::fonts;
use crate::i18n::{self, Language};
use crate::icons::IconCache;
use crate::shell::overlays::{Picker, PickerEvent, PickerKind};
use crate::shell::{Actions, Kit, UiState, chrome, empty_state, overlays, panels};
use crate::theme::{Theme, ThemeId, Tok};
use crate::{editor_widget, input};

/// Env overrides for manual checks and snapshots (not persisted).
pub const LANG_ENV: &str = "NOTOIR_LANG";
pub const THEME_ENV: &str = "NOTOIR_THEME";

pub struct NotoirApp {
    pub core: AppCore,
    pub ui: UiState,
    theme_id: ThemeId,
    theme: Theme,
    icons: IconCache,
    started: Instant,
    /// System CJK scripts included in the last font install.
    fonts_installed: Option<Vec<Script>>,
    /// CJK scripts seen in editor text this session.
    cjk_seen: fonts::CjkUse,
    wait_for_fonts: bool,
}

impl NotoirApp {
    /// Production entry: paths and overrides from the environment.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let paths = notoir_platform::AppPaths::resolve()
            .or_else(|_| {
                notoir_platform::AppPaths::resolve_with(Some(
                    std::env::temp_dir().join("notoir").into(),
                ))
            })
            .expect("app directories");
        let mut opts = StartOptions::new(paths);
        opts.os_locale = notoir_platform::os_locale();
        opts.language_override = std::env::var(LANG_ENV)
            .ok()
            .as_deref()
            .and_then(Language::from_code);
        opts.theme_override = std::env::var(THEME_ENV)
            .ok()
            .as_deref()
            .and_then(ThemeId::from_id);
        Self::with_options(&cc.egui_ctx, opts)
    }

    pub fn with_options(ctx: &egui::Context, opts: StartOptions) -> Self {
        let repaint = ctx.clone();
        let core = AppCore::new(&opts, move || repaint.request_repaint());
        let theme_id = core.theme(system_dark(ctx));
        let mut app = Self {
            core,
            ui: UiState::default(),
            theme_id,
            theme: theme_id.load(),
            icons: IconCache::default(),
            started: Instant::now(),
            fonts_installed: None,
            cjk_seen: fonts::CjkUse::default(),
            wait_for_fonts: opts.wait_for_fonts,
        };
        app.apply_language(ctx);
        app.theme.apply(ctx);
        // Ctrl/Cmd +/-/0 are Notoir's editor zoom commands, not UI scaling.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        app
    }

    pub fn theme_id(&self) -> ThemeId {
        self.theme_id
    }

    fn apply_language(&mut self, ctx: &egui::Context) {
        i18n::set_language(self.core.language);
        self.ensure_fonts(ctx);
    }

    /// Installs fonts: bundled faces plus system CJK faces for the UI
    /// language and any CJK text the editor showed. Faces load in the
    /// background and are added when ready.
    fn ensure_fonts(&mut self, ctx: &egui::Context) {
        let mut scripts: Vec<Script> = fonts::script_for(self.core.language).into_iter().collect();
        scripts.extend(self.cjk_seen.scripts(self.core.language));
        scripts.sort();
        scripts.dedup();
        let mut faces = Vec::new();
        for script in scripts {
            let face = if self.wait_for_fonts {
                Some(system_fonts::cjk_font(script))
            } else {
                let ready = system_fonts::ready(script);
                if ready.is_none() {
                    let repaint = ctx.clone();
                    system_fonts::request(script, move || repaint.request_repaint());
                }
                ready
            };
            if let Some(Some(f)) = face {
                faces.push(f);
            }
        }
        let want: Vec<Script> = faces.iter().map(|f| f.script).collect();
        if self.fonts_installed.as_ref() != Some(&want) {
            fonts::install(ctx, &faces);
            self.fonts_installed = Some(want);
        }
    }

    fn set_theme(&mut self, ctx: &egui::Context, id: ThemeId) {
        if id != self.theme_id {
            self.theme_id = id;
            self.theme = id.load();
            self.theme.apply(ctx);
            self.core
                .workbench
                .events
                .emit(notoir_core::AppEvent::ThemeChanged {
                    theme: id.id().to_owned(),
                });
        }
    }

    fn apply_effects(&mut self, ctx: &egui::Context, centre_was_open: bool) {
        for effect in std::mem::take(&mut self.core.effects) {
            match effect {
                Effect::ApplyTheme => {
                    let id = self.core.theme(system_dark(ctx));
                    self.set_theme(ctx, id);
                }
                Effect::ApplyLanguage => {
                    self.apply_language(ctx);
                    let lang = self.core.language.code().to_owned();
                    self.core
                        .workbench
                        .events
                        .emit(notoir_core::AppEvent::LanguageChanged { lang });
                }
                Effect::ToggleFullScreen => {
                    let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
                }
                Effect::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Effect::OpenThemePicker => {
                    self.ui.picker = Some(Picker::new(PickerKind::Theme, &self.core))
                }
                Effect::OpenLanguagePicker => {
                    self.ui.picker = Some(Picker::new(PickerKind::Language, &self.core))
                }
                Effect::ShowAbout => self.ui.about_open = true,
                Effect::FocusEditor(view) => self.ui.focus_editor = Some(view),
                Effect::CopyText(text) => ctx.copy_text(text),
                Effect::ToggleNotificationCentre => {
                    self.ui.notification_centre_open = !centre_was_open;
                    if self.ui.notification_centre_open {
                        self.core.notifications.mark_read();
                    }
                }
            }
        }
    }

    fn handle_picker(&mut self, ctx: &egui::Context, top_offset: f32) {
        let Some(picker) = self.ui.picker.as_mut() else {
            return;
        };
        let kind = picker.kind;
        let mut kit = Kit {
            theme: &self.theme,
            icons: &mut self.icons,
        };
        let Some(event) = picker.show(ctx, &mut kit, top_offset) else {
            return;
        };
        match event {
            PickerEvent::Preview(value) => {
                if let Some(choice) = ThemeChoice::from_setting(&value) {
                    self.set_theme(ctx, choice.resolve(system_dark(ctx)));
                }
            }
            PickerEvent::Accept(value) => {
                self.ui.picker = None;
                let inv = match kind {
                    PickerKind::Theme => {
                        CommandInvocation::new("preferences.changeTheme", InvocationSource::Palette)
                            .with_args(json!({ "theme": value }))
                    }
                    PickerKind::Language => CommandInvocation::new(
                        "preferences.changeLanguage",
                        InvocationSource::Palette,
                    )
                    .with_args(json!({ "language": value })),
                };
                self.core.enqueue(inv);
            }
            PickerEvent::Cancel => {
                self.ui.picker = None;
                let id = self.core.theme(system_dark(ctx));
                self.set_theme(ctx, id);
            }
        }
    }
}

fn system_dark(ctx: &egui::Context) -> bool {
    ctx.system_theme() != Some(egui::Theme::Light)
}

impl eframe::App for NotoirApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.core.now_ms = self.started.elapsed().as_millis() as u64;
        let now = self.core.now_ms;
        self.core.pump_io();
        self.ensure_fonts(&ctx);
        if self.ui.picker.is_none() {
            // Theme "auto" follows the OS.
            let id = self.core.theme(system_dark(&ctx));
            self.set_theme(&ctx, id);
        }
        self.core.notifications.tick(now);

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let context = self.core.context(fullscreen);
        if !(self.ui.about_open || self.ui.picker.is_some()) {
            input::dispatch_keys(&ctx, &mut self.core, &context);
        }
        let centre_was_open = self.ui.notification_centre_open;

        let mut actions = Actions::default();
        let mut top_offset = chrome::MENU_BAR_HEIGHT;
        let mut bottom_offset = 0.0;
        {
            let mut kit = Kit {
                theme: &self.theme,
                icons: &mut self.icons,
            };
            chrome::menu_bar(ui, &mut kit, &self.core, &context, &mut actions);
            if self.core.toolbar_visible() {
                chrome::toolbar(ui, &mut kit, &self.core, &context, &mut actions);
                top_offset += chrome::TOOLBAR_HEIGHT;
            }
            if self.core.status_bar_visible() {
                chrome::status_bar(ui, &mut kit, &self.core, &mut actions);
                bottom_offset += chrome::STATUS_BAR_HEIGHT;
            }
            let sizes = panels::show(ui, &mut kit, &self.core, &mut actions);
            let editor_fill = kit.color(Tok::BgEditor);
            let active = self.core.workbench.active_view();
            let options = active.map(|v| self.core.editor_options(v));
            let focus_request = self.ui.focus_editor.take();
            let mut editor_focused = false;
            let mut zoom_steps = 0;
            egui::CentralPanel::no_frame()
                .frame(egui::Frame::new().fill(editor_fill))
                .show(ui, |ui| match (active, options) {
                    (Some(view), Some(opts)) => {
                        let out = editor_widget::show(
                            ui,
                            &mut kit,
                            &mut self.core.workbench,
                            &mut self.ui.editors,
                            view,
                            &opts,
                            focus_request == Some(view),
                            now as f64 / 1000.0,
                        );
                        editor_focused = out.focused;
                        self.cjk_seen.merge(out.cjk);
                        zoom_steps = out.zoom_steps;
                    }
                    _ => empty_state::show(ui, &mut kit, self.core.platform),
                });
            self.core.editor_focused = editor_focused;
            match zoom_steps.cmp(&0) {
                std::cmp::Ordering::Greater => actions.push(CommandInvocation::new(
                    "view.zoomIn",
                    InvocationSource::Internal,
                )),
                std::cmp::Ordering::Less => actions.push(CommandInvocation::new(
                    "view.zoomOut",
                    InvocationSource::Internal,
                )),
                std::cmp::Ordering::Equal => {}
            }
            let workbench = &self.core.workbench;
            self.ui.editors.retain(|v| workbench.view(v).is_some());

            let (hovered, dismissed) = overlays::toasts(
                &ctx,
                &mut kit,
                &self.core,
                &mut self.ui,
                bottom_offset,
                &mut actions,
            );
            overlays::notification_centre(
                &ctx,
                &mut kit,
                &self.core,
                &mut self.ui,
                bottom_offset,
                &mut actions,
            );
            overlays::about(&ctx, &mut kit, &mut self.ui);
            for id in hovered {
                self.core.notifications.hold(id, now);
            }
            for id in dismissed {
                self.core.notifications.dismiss(id);
            }
            for (area, size) in [
                (PanelArea::Primary, sizes.primary),
                (PanelArea::Secondary, sizes.secondary),
                (PanelArea::Bottom, sizes.bottom),
            ] {
                if let Some(s) = size {
                    self.core.panels.set_size(area, s);
                }
            }
        }
        self.handle_picker(&ctx, top_offset);

        for inv in actions.0 {
            self.core.enqueue(inv);
        }
        self.core.run_queued(&context);
        self.apply_effects(&ctx, centre_was_open);
        for event in self.core.workbench.events.drain() {
            tracing::trace!(?event, "event");
        }
        if let Some(expiry) = self.core.notifications.next_expiry() {
            ctx.request_repaint_after(std::time::Duration::from_millis(expiry.saturating_sub(now)));
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(self.theme.color(Tok::BgApp)).to_array()
    }
}
