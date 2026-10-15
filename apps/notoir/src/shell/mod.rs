//! Window chrome (docs/specs/17, 18, 19): menu bar, toolbar, status bar,
//! panel host, empty state and overlays. Rendering only — every action is
//! returned as a [`CommandInvocation`] for the command layer.

use egui::{Color32, Response, Sense, Vec2};
use notoir_core::command::CommandInvocation;

use crate::icons::{Icon, IconCache};
use crate::theme::{ICON, Theme, Tok};

pub mod chrome;
pub mod empty_state;
pub mod explorer;
pub mod find_widget;
pub mod json_tree;
pub mod layout;
pub mod md_preview;
pub mod overlays;
pub mod panels;
pub mod prompts;
pub mod recovery_ui;
pub mod settings_ui;
pub mod tabs;
pub mod welcome;

/// Theme + icon cache passed to every rendering function.
pub struct Kit<'a> {
    pub theme: &'a Theme,
    pub icons: &'a mut IconCache,
}

impl Kit<'_> {
    pub fn color(&self, tok: Tok) -> Color32 {
        self.theme.color(tok)
    }

    pub fn icon(
        &mut self,
        ctx: &egui::Context,
        icon: Icon,
        size: f32,
        color: Color32,
    ) -> egui::Image<'static> {
        self.icons.image(ctx, icon, size, color)
    }
}

/// UI-only state that never leaves the app crate.
#[derive(Default)]
pub struct UiState {
    pub picker: Option<overlays::Picker>,
    pub about_open: bool,
    pub notification_centre_open: bool,
    /// Notification whose details are expanded.
    pub expanded_detail: Option<u64>,
    pub editors: crate::editor_widget::EditorCaches,
    /// Editor that should take keyboard focus next frame.
    pub focus_editor: Option<notoir_core::ids::ViewId>,
    pub tab_drag: Option<layout::TabDrag>,
    pub explorer: explorer::ExplorerUi,
    /// Active document last frame (Explorer auto-reveal).
    pub last_active_doc: Option<notoir_core::ids::DocumentId>,
    pub window_title: String,
    pub window_focused: bool,
    /// Last full-screen request (tests; the OS reports the real state).
    pub fullscreen_requested: Option<bool>,
    pub previews: md_preview::Previews,
    pub json_trees: json_tree::JsonTrees,
}

/// Collects invocations produced while rendering one frame.
#[derive(Default)]
pub struct Actions(pub Vec<CommandInvocation>);

impl Actions {
    pub fn push(&mut self, inv: CommandInvocation) {
        self.0.push(inv);
    }
}

/// 28×28 icon-only button with tooltip (spec 21: hit target ≥ icon).
pub fn icon_button(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    icon: Icon,
    tooltip: &str,
    active: bool,
    enabled: bool,
) -> Response {
    icon_button_sized(ui, kit, icon, tooltip, active, enabled, 28.0)
}

/// Icon button with a custom square size (status bar: 22).
pub fn icon_button_sized(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    icon: Icon,
    tooltip: &str,
    active: bool,
    enabled: bool,
    size: f32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::splat(size),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if active {
        ui.painter()
            .rect_filled(rect, crate::theme::RADIUS, kit.color(Tok::AccentSoft));
    } else if resp.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, crate::theme::RADIUS, kit.color(Tok::BgPanelRaised));
    }
    let color = if !enabled {
        kit.color(Tok::TextDisabled)
    } else if active {
        kit.color(Tok::Accent)
    } else {
        kit.color(Tok::TextSecondary)
    };
    let icon_size = if size < 24.0 {
        crate::theme::ICON_SM
    } else {
        ICON
    };
    let image = kit.icon(ui.ctx(), icon, icon_size, color);
    image.paint_at(
        ui,
        egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size)),
    );
    // Screen readers (and UI tests) see the tooltip as the button's label.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    resp.on_hover_text(tooltip)
}

/// Uppercase 11 px section label (the only such style, spec 21).
pub fn section_label(ui: &mut egui::Ui, kit: &Kit, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(crate::fonts::extrabold(11.0))
            .color(kit.color(Tok::TextTertiary)),
    );
}
