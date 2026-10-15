//! `NotoirApp`: the eframe application and its frame loop.
//!
//! Per frame: pump background results → keyboard → render chrome (collecting
//! command invocations) → run queued commands → apply effects that need the
//! egui context → drain events.

use std::time::Instant;

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::panels::PanelArea;
use notoir_platform::system_fonts::{self, Script};

use crate::core::{AppCore, Effect, StartOptions, ThemeChoice};
use crate::fonts;
use crate::i18n::{self, Language};
use crate::icons::IconCache;
use crate::input;
use crate::shell::overlays::{Picker, PickerEvent};
use crate::shell::{self, Actions, Kit, UiState, chrome, overlays, panels};
use crate::theme::{Theme, ThemeId, Tok};

/// Env overrides for manual checks and snapshots (not persisted).
pub const LANG_ENV: &str = "NOTOIR_LANG";
pub const THEME_ENV: &str = "NOTOIR_THEME";

/// What `main` hands the app: command-line files and forwarded launches.
pub struct Launch {
    pub open_paths: Vec<(std::path::PathBuf, Option<(usize, usize)>)>,
    /// No paths and not `--new-window`: restore the last session.
    pub restore_session: bool,
    pub instance_rx:
        Option<std::sync::mpsc::Receiver<notoir_platform::single_instance::InstanceRequest>>,
}

pub struct NotoirApp {
    pub core: AppCore,
    pub ui: UiState,
    theme_id: ThemeId,
    /// The theme being drawn (tests check what the UI shows).
    pub theme: Theme,
    icons: IconCache,
    started: Instant,
    /// System CJK scripts included in the last font install.
    fonts_installed: Option<Vec<Script>>,
    /// CJK scripts seen in editor text this session.
    cjk_seen: fonts::CjkUse,
    wait_for_fonts: bool,
    instance_rx:
        Option<std::sync::mpsc::Receiver<notoir_platform::single_instance::InstanceRequest>>,
}

impl NotoirApp {
    /// Production entry: paths and overrides from the environment.
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let paths = notoir_platform::AppPaths::resolve()
            .or_else(|_| {
                notoir_platform::AppPaths::resolve_with(Some(
                    std::env::temp_dir().join("notoir").into(),
                ))
            })
            .expect("app directories");
        let mut opts = StartOptions::new(paths);
        opts.open_paths = launch.open_paths;
        opts.restore_session = launch.restore_session;
        opts.first_run = true;
        opts.os_locale = notoir_platform::os_locale();
        opts.language_override = std::env::var(LANG_ENV)
            .ok()
            .as_deref()
            .and_then(Language::from_code);
        opts.theme_override = std::env::var(THEME_ENV)
            .ok()
            .as_deref()
            .and_then(ThemeId::from_id);
        let mut app = Self::with_options(&cc.egui_ctx, opts);
        app.instance_rx = launch.instance_rx;
        app
    }

    pub fn with_options(ctx: &egui::Context, opts: StartOptions) -> Self {
        let repaint = ctx.clone();
        let mut core = AppCore::new(&opts, move || repaint.request_repaint());
        core.startup(&opts.open_paths, opts.restore_session);
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
            instance_rx: None,
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
                Effect::SetFullScreen(on) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
                    self.ui.fullscreen_requested = Some(on);
                }
                Effect::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Effect::OpenPicker(kind) => self.ui.picker = Some(Picker::new(kind, &self.core)),
                Effect::ShowAbout => self.ui.about_open = true,
                Effect::FocusEditor(view) => self.ui.focus_editor = Some(view),
                Effect::CopyText(text) => ctx.copy_text(text),
                Effect::FocusWindow => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
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
                let picker = self.ui.picker.take();
                let inv = Picker::accept(kind, &value);
                match picker.and_then(|p| p.context) {
                    Some(ctx) if kind == crate::shell::overlays::PickerKind::Commands => {
                        if let Some(db) = self.core.state_db.as_ref() {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_millis() as i64);
                            let _ = db.touch_palette(&value, now);
                        }
                        // Focus returns to the editor the palette came from.
                        if let Some(v) = self.core.workbench.active_view() {
                            self.ui.focus_editor = Some(v);
                        }
                        self.core.queue_with_context.push((inv, ctx));
                    }
                    _ => self.core.enqueue(inv),
                }
            }
            PickerEvent::Cancel => {
                if kind == crate::shell::overlays::PickerKind::Commands
                    && let Some(v) = self.core.workbench.active_view()
                {
                    self.ui.focus_editor = Some(v);
                }
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
        let forwarded: Vec<_> = self
            .instance_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for req in forwarded {
            self.core.handle_instance_request(req);
        }
        self.ensure_fonts(&ctx);
        if self.ui.picker.is_none() {
            // Theme "auto" follows the OS.
            let id = self.core.theme(system_dark(&ctx));
            self.set_theme(&ctx, id);
        }
        self.core.notifications.tick(now);
        self.core.flush_scratchpads(false);
        self.core.workspace_tick();
        self.core.recovery_tick();
        self.core.autosave_tick();
        self.core.sync_doc_watches();
        self.core.session.geometry = ctx.input(|i| {
            let v = i.viewport();
            v.outer_rect.map(|r| crate::session::WindowGeometry {
                x: r.left(),
                y: r.top(),
                width: v.inner_rect.map_or(r.width(), |r| r.width()),
                height: v.inner_rect.map_or(r.height(), |r| r.height()),
                maximized: v.maximized.unwrap_or(false),
                fullscreen: v.fullscreen.unwrap_or(false),
            })
        });
        self.core.write_session(true);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.ui.window_focused && !focused {
            self.core.window_focus_lost();
        }
        self.ui.window_focused = focused;
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.core.request_quit() {
                self.core.shutdown();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let context = self.core.context(fullscreen);
        if !(self.ui.about_open
            || self.ui.picker.is_some()
            || self.core.find.goto.is_some()
            || self.core.settings_ui.recording.is_some()
            || !self.core.files.prompts.is_empty())
        {
            input::dispatch_keys(&ctx, &mut self.core, &context);
        }
        let centre_was_open = self.ui.notification_centre_open;

        let mut actions = Actions::default();
        let mut prompt_answer = None;
        let mut top_offset = chrome::MENU_BAR_HEIGHT;
        let mut bottom_offset = 0.0;
        {
            let mut kit = Kit {
                theme: &self.theme,
                icons: &mut self.icons,
            };
            let zen = self.core.zen.is_some();
            // Zen Mode: the menu bar shows only while Alt is held (Win/Linux).
            if !zen || ctx.input(|i| i.modifiers.alt) {
                chrome::menu_bar(ui, &mut kit, &self.core, &context, &mut actions);
            } else {
                top_offset = 0.0;
            }
            if self.core.toolbar_visible() && !zen {
                chrome::toolbar(ui, &mut kit, &self.core, &context, &mut actions);
                top_offset += chrome::TOOLBAR_HEIGHT;
            }
            if self.core.status_bar_visible() && !zen {
                chrome::status_bar(ui, &mut kit, &self.core, &mut actions);
                bottom_offset += chrome::STATUS_BAR_HEIGHT;
            }
            let sizes = panels::show(
                ui,
                &mut kit,
                &mut self.core,
                &context,
                &mut self.ui.explorer,
                &mut actions,
            );
            let editor_fill = kit.color(Tok::BgEditor);
            let focus_request = self.ui.focus_editor.take();
            let mut layout_out = shell::layout::LayoutOutput::default();
            egui::CentralPanel::no_frame()
                .frame(egui::Frame::new().fill(editor_fill))
                .show(ui, |ui| {
                    layout_out = shell::layout::show(
                        ui,
                        &mut kit,
                        &mut self.core,
                        &mut self.ui.editors,
                        &mut self.ui.tab_drag,
                        &mut self.ui.explorer.drag,
                        focus_request,
                        &context,
                        &mut actions,
                        now as f64 / 1000.0,
                    );
                });
            let editor_focused = layout_out.editor_focused;
            let zoom_steps = layout_out.zoom_steps;
            self.cjk_seen.merge(layout_out.cjk);
            if let Some(view) = layout_out.activate {
                let _ = self.core.workbench.activate_view(view);
            }
            // One-frame scroll requests were consumed by the editors.
            self.core.find.center.clear();
            self.core.find.scroll.clear();
            for (path, index, delta, min) in layout_out.resizes {
                self.core.workbench.layout.resize(&path, index, delta, min);
            }
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
            shell::recovery_ui::show(&ctx, &mut kit, &self.core, top_offset, &mut actions);
            overlays::about(&ctx, &mut kit, &mut self.ui);
            if let Some(answer) = shell::prompts::show(&ctx, &mut kit, &self.core) {
                prompt_answer = Some(answer);
            }
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
        {
            let mut kit = Kit {
                theme: &self.theme,
                icons: &mut self.icons,
            };
            shell::overlays::goto_line(&ctx, &mut kit, &mut self.core, top_offset);
        }

        for inv in actions.0 {
            self.core.enqueue(inv);
        }
        self.core.run_queued(&context);
        self.apply_effects(&ctx, centre_was_open);
        // Typing on the empty state starts a Scratchpad (spec 17).
        if self.core.workbench.active_view().is_none()
            && !ctx.egui_wants_keyboard_input()
            && self.ui.picker.is_none()
            && !self.ui.about_open
            && self.core.files.prompts.is_empty()
        {
            let typed: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            if !typed.is_empty() {
                self.core.new_scratchpad_with_text(&typed);
            }
        }
        if let Some(answer) = prompt_answer {
            self.core.answer_prompt(answer);
        }
        // A finished drag that nothing took.
        if self.ui.explorer.drag.is_some() && !ctx.input(|i| i.pointer.primary_down()) {
            self.ui.explorer.drag = None;
        }
        let active_doc = self.core.workbench.active_document();
        if active_doc != self.ui.last_active_doc {
            let previous = std::mem::replace(&mut self.ui.last_active_doc, active_doc);
            self.core.autosave_focus_change(previous);
            self.core.auto_reveal(active_doc);
        }
        let title = match self.core.workspace_name() {
            Some(name) => format!("{name} — Notoir"),
            None => "Notoir".to_owned(),
        };
        if title != self.ui.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.window_title = title;
        }
        for event in self.core.workbench.events.drain() {
            match &event {
                notoir_core::AppEvent::DocumentChanged { doc, .. } => {
                    self.core.scratchpad_changed(*doc);
                    self.core.recovery_doc_changed(*doc);
                    self.core.autosave_doc_changed(*doc);
                }
                notoir_core::AppEvent::DocumentSaved { doc } => {
                    if self
                        .core
                        .workbench
                        .document(*doc)
                        .and_then(|d| d.path())
                        .is_some_and(|p| self.core.is_config_file(p))
                    {
                        self.core.reload_config();
                    }
                    self.core.recovery_doc_changed(*doc);
                    self.core.autosave_due.remove(doc);
                }
                notoir_core::AppEvent::DocumentClosed { doc } => {
                    self.core.recovery_forget(*doc);
                    self.core.banners.remove(doc);
                    self.core.autosave_due.remove(doc);
                }
                _ => {}
            }
            tracing::trace!(?event, "event");
        }
        if let Some(due) = self.core.next_scratch_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now) + 1,
            ));
        }
        for due in [self.core.recovery_next_due(), self.core.autosave_next_due()]
            .into_iter()
            .flatten()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now) + 1,
            ));
        }
        if let Some(due) = self.core.workspace_next_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now) + 1,
            ));
        }
        if let Some(expiry) = self.core.notifications.next_expiry() {
            ctx.request_repaint_after(std::time::Duration::from_millis(expiry.saturating_sub(now)));
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(self.theme.color(Tok::BgApp)).to_array()
    }
}
