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
    pub instance_rx: Option<
        std::sync::mpsc::Receiver<(
            notoir_platform::single_instance::InstanceRequest,
            Option<notoir_platform::single_instance::Waiter>,
        )>,
    >,
}

pub struct NotoirApp {
    pub core: AppCore,
    pub ui: UiState,
    /// Id of the applied theme and the custom-theme load it came from.
    theme_key: (String, u64),
    /// The theme being drawn (tests check what the UI shows).
    pub theme: Theme,
    icons: IconCache,
    started: Instant,
    /// System CJK scripts included in the last font install.
    fonts_installed: Option<Vec<Script>>,
    /// CJK scripts seen in editor text this session.
    cjk_seen: fonts::CjkUse,
    wait_for_fonts: bool,
    ctrl_click: crate::input::CtrlClick,
    instance_rx: Option<
        std::sync::mpsc::Receiver<(
            notoir_platform::single_instance::InstanceRequest,
            Option<notoir_platform::single_instance::Waiter>,
        )>,
    >,
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
        let theme = core.load_theme(&theme_id);
        let theme_key = (theme_id, core.themes_generation);
        let mut app = Self {
            core,
            ui: UiState::default(),
            theme_key,
            theme,
            icons: IconCache::default(),
            started: Instant::now(),
            fonts_installed: None,
            cjk_seen: fonts::CjkUse::default(),
            wait_for_fonts: opts.wait_for_fonts,
            ctrl_click: Default::default(),
            instance_rx: None,
        };
        app.apply_language(ctx);
        app.theme.apply(ctx);
        // Ctrl/Cmd +/-/0 are Notoir's editor zoom commands, not UI scaling.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        app
    }

    /// Id of the applied theme.
    pub fn theme_id(&self) -> &str {
        &self.theme_key.0
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
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

    fn set_theme(&mut self, ctx: &egui::Context, id: String) {
        let key = (id, self.core.themes_generation);
        if key != self.theme_key {
            self.theme = self.core.load_theme(&key.0);
            self.theme.apply(ctx);
            tracing::debug!(requested = %key.0, applied = %self.theme.id, "theme applied");
            self.core
                .workbench
                .events
                .emit(notoir_core::AppEvent::ThemeChanged {
                    theme: key.0.clone(),
                });
            self.theme_key = key;
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
                Effect::OpenPicker(kind) => {
                    if kind == crate::shell::overlays::PickerKind::Symbols
                        && let Some(d) = self.core.workbench.active_document()
                    {
                        self.core.structure(d);
                    }
                    self.ui.picker = Some(Picker::new(kind, &self.core));
                }
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
            PickerEvent::Preview(value) if kind == crate::shell::overlays::PickerKind::Symbols => {
                // Preview the location; Enter keeps it, Esc goes back.
                if let (Some((a, b)), Some(v)) = (
                    crate::shell::overlays::parse_range(&value),
                    self.core.workbench.active_view(),
                ) {
                    let sel = notoir_editor::Selection::single(notoir_editor::Range::new(a, b));
                    let _ = self.core.workbench.set_selection(v, sel);
                    self.core.find.center.push(v);
                }
            }
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
            PickerEvent::Switch(to, query) => {
                if to == crate::shell::overlays::PickerKind::Symbols
                    && let Some(d) = self.core.workbench.active_document()
                {
                    self.core.structure(d);
                }
                let mut p = Picker::new(to, &self.core);
                p.from_quick_open = to != crate::shell::overlays::PickerKind::QuickOpen;
                p.set_query(query);
                self.ui.picker = Some(p);
            }
            PickerEvent::GotoLine(text) => {
                self.ui.picker = None;
                if let Some(v) = self.core.workbench.active_view() {
                    self.core.open_goto_line(v);
                    if !text.trim().is_empty() {
                        self.core.goto_preview(text);
                    }
                }
            }
            PickerEvent::Cancel => {
                if let Some((v, sel, scroll)) =
                    self.ui.picker.as_mut().and_then(|p| p.restore.take())
                {
                    let _ = self.core.workbench.set_selection(v, sel);
                    self.core.find.scroll.insert(v, scroll);
                    self.ui.focus_editor = Some(v);
                }
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
        for (req, waiter) in forwarded {
            self.core.handle_instance_request(req, waiter);
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
        self.core.search_tick();
        self.core.compare_tick();
        self.core.git_tick();
        self.core.mcp_tick();
        self.core.plugins_tick();
        self.core.notes_tick();
        self.core.text_index_tick();
        self.core.updates_tick();
        if let Some(due) = self.core.notes_next_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now).max(1),
            ));
        }
        if !self.core.cli_waits.is_empty() {
            self.core.cli_wait_tick();
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if let Some(due) = self.core.plugins_next_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now).max(1),
            ));
        }
        if !self.core.mcp.highlights.is_empty() {
            // MCP edit highlights fade out.
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        if let Some(due) = self.core.mcp_next_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now).max(1),
            ));
        }
        if self.core.git_gutter_pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                crate::git::GUTTER_DEBOUNCE_MS,
            ));
        }
        if let Some(due) = self.core.git_next_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                due.saturating_sub(now).max(1),
            ));
        }
        if self.core.compare_needs_repaint() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                crate::compare::DEBOUNCE_MS,
            ));
        }
        // Markdown preview images decoded in the background become textures.
        for (path, data) in std::mem::take(&mut self.core.markdown.decoded) {
            let tex = match data {
                crate::markdown::ImageData::Ready {
                    width,
                    height,
                    rgba,
                } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    let handle = ctx.load_texture(
                        path.display().to_string(),
                        img,
                        egui::TextureOptions::LINEAR,
                    );
                    shell::md_preview::ImageTex::Ready(
                        handle,
                        egui::vec2(width as f32, height as f32),
                    )
                }
                crate::markdown::ImageData::Missing(why) => {
                    shell::md_preview::ImageTex::Missing(why)
                }
            };
            self.ui.previews.images.insert(path, tex);
        }
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
        if !self.ui.window_focused && focused {
            self.core.git_rediscover_soon();
        }
        self.ui.window_focused = focused;
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.core.request_quit() {
                self.core.shutdown();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        // `[[` completion keys go to the popup, not the editor or keymap.
        if self.core.notes.completion.is_some() {
            for key in [
                egui::Key::ArrowDown,
                egui::Key::ArrowUp,
                egui::Key::Enter,
                egui::Key::Tab,
                egui::Key::Escape,
            ] {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                    self.core.notes_completion_key(key);
                }
            }
        }
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let context = self.core.context(fullscreen);
        if !(self.ui.about_open
            || self.ui.picker.is_some()
            || self.core.find.goto.is_some()
            || self.core.settings_ui.recording.is_some()
            || !self.core.files.prompts.is_empty()
            || self.core.mcp.dialog.is_some()
            || self.core.plugins.review.is_some()
            || self.core.plugins.uninstall_confirm.is_some()
            || !self.core.plugins.process_prompts.is_empty())
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
                &mut panels::PanelUis {
                    explorer: &mut self.ui.explorer,
                    outline: &mut self.ui.outline,
                    scm: &mut self.ui.scm,
                },
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
                        &mut self.ui.previews,
                        &mut self.ui.json_trees,
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
            shell::notes_ui::completion(&ctx, &mut kit, &self.core, &self.ui.editors, &mut actions);
            shell::mcp_ui::popover(&ctx, &mut kit, &self.core, bottom_offset, &mut actions);
            shell::mcp_ui::dialogs(
                &ctx,
                &mut kit,
                &self.core,
                &mut self.ui.mcp_client_name,
                &mut actions,
            );
            shell::plugins_ui::review(&ctx, &mut kit, &self.core, &mut actions);
            shell::plugins_ui::process_prompt(&ctx, &mut kit, &self.core, &mut actions);
            shell::plugins_ui::uninstall_confirm(&ctx, &mut kit, &self.core, &mut actions);
            // Prompts show even in Zen Mode: a client is waiting on them.
            shell::mcp_ui::ask_bar(&ctx, &mut kit, &mut self.core, bottom_offset, &mut actions);
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

        // Mouse back/forward buttons (spec 15).
        let (x1, x2) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.pointer.button_pressed(egui::PointerButton::Extra2),
            )
        });
        if x1 {
            actions.push(CommandInvocation::new(
                "navigate.back",
                InvocationSource::Internal,
            ));
        }
        if x2 {
            actions.push(CommandInvocation::new(
                "navigate.forward",
                InvocationSource::Internal,
            ));
        }
        for inv in actions.0 {
            self.core.enqueue(inv);
        }
        self.core.run_queued(&context);
        self.apply_effects(&ctx, centre_was_open);
        self.core.nav_track();
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
            self.core.plugins_event(&event);
            match &event {
                notoir_core::AppEvent::DocumentChanged { doc, .. } => {
                    self.core.notes_doc_changed(*doc);
                    self.core.scratchpad_changed(*doc);
                    self.core.recovery_doc_changed(*doc);
                    self.core.autosave_doc_changed(*doc);
                }
                notoir_core::AppEvent::DocumentSaved { doc } => {
                    self.core.git_refresh_soon();
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
        if let Some(due) = self.core.search_next_due() {
            ctx.request_repaint_after(std::time::Duration::from_millis(due.saturating_sub(now)));
        }
        if let Some(expiry) = self.core.notifications.next_expiry() {
            ctx.request_repaint_after(std::time::Duration::from_millis(expiry.saturating_sub(now)));
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if cfg!(target_os = "macos") {
            self.ctrl_click.apply(raw_input);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(self.theme.color(Tok::BgApp)).to_array()
    }
}
