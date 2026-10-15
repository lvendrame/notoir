//! Generic panel layout (docs/specs/17-shell-layout-and-panels.md §Panel Model):
//! primary sidebar, secondary sidebar and bottom panel, each showing one
//! panel at a time. Independent of how the app renders it.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PanelId {
    Explorer,
    Search,
    SourceControl,
    Outline,
    Plugins,
    MarkdownPreview,
    JsonTree,
    McpActivity,
    Output,
    Problems,
    GitOutput,
    PluginOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelArea {
    Primary,
    Secondary,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

impl PanelId {
    pub fn default_area(self) -> PanelArea {
        match self {
            PanelId::Explorer | PanelId::Search | PanelId::SourceControl | PanelId::Plugins => {
                PanelArea::Primary
            }
            PanelId::Outline | PanelId::MarkdownPreview | PanelId::JsonTree => PanelArea::Secondary,
            PanelId::McpActivity
            | PanelId::Output
            | PanelId::Problems
            | PanelId::GitOutput
            | PanelId::PluginOutput => PanelArea::Bottom,
        }
    }

    /// Areas the panel may be moved to.
    pub fn allowed_areas(self) -> &'static [PanelArea] {
        use PanelArea::*;
        match self {
            PanelId::Explorer | PanelId::Search | PanelId::SourceControl | PanelId::Plugins => {
                &[Primary, Secondary]
            }
            PanelId::Outline | PanelId::McpActivity => &[Primary, Secondary, Bottom],
            PanelId::MarkdownPreview | PanelId::JsonTree => &[Primary, Secondary],
            PanelId::Output | PanelId::Problems | PanelId::GitOutput | PanelId::PluginOutput => {
                &[Bottom]
            }
        }
    }

    /// i18n key of the panel title.
    pub fn title_key(self) -> &'static str {
        match self {
            PanelId::Explorer => "panels:explorer.title",
            PanelId::Search => "panels:search.title",
            PanelId::SourceControl => "panels:sourceControl.title",
            PanelId::Outline => "panels:outline.title",
            PanelId::Plugins => "panels:plugins.title",
            PanelId::MarkdownPreview => "panels:markdownPreview.title",
            PanelId::JsonTree => "panels:jsonTree.title",
            PanelId::McpActivity => "panels:mcpActivity.title",
            PanelId::Output => "panels:output.title",
            PanelId::Problems => "panels:problems.title",
            PanelId::GitOutput => "panels:gitOutput.title",
            PanelId::PluginOutput => "panels:pluginOutput.title",
        }
    }
}

impl PanelArea {
    pub fn min_size(self) -> f32 {
        match self {
            PanelArea::Primary | PanelArea::Secondary => 170.0,
            PanelArea::Bottom => 100.0,
        }
    }

    pub fn default_size(self) -> f32 {
        match self {
            PanelArea::Primary | PanelArea::Secondary => 260.0,
            PanelArea::Bottom => 220.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaState {
    pub panels: Vec<PanelId>,
    pub active: Option<PanelId>,
    pub visible: bool,
    /// Width for sidebars, height for the bottom panel (logical px).
    pub size: f32,
    /// Bottom panel only: takes over the editor area.
    pub maximized: bool,
}

impl AreaState {
    fn new(area: PanelArea) -> Self {
        Self {
            panels: Vec::new(),
            active: None,
            visible: false,
            size: area.default_size(),
            maximized: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelLayout {
    pub primary_side: Side,
    pub primary: AreaState,
    pub secondary: AreaState,
    pub bottom: AreaState,
    pub activity_bar_visible: bool,
}

impl PanelLayout {
    /// Fresh-install layout for the registered panels: primary sidebar visible
    /// (first panel active), secondary sidebar and bottom panel hidden.
    pub fn new(registered: &[PanelId]) -> Self {
        let mut layout = Self {
            primary_side: Side::Left,
            primary: AreaState::new(PanelArea::Primary),
            secondary: AreaState::new(PanelArea::Secondary),
            bottom: AreaState::new(PanelArea::Bottom),
            activity_bar_visible: true,
        };
        for &id in registered {
            let area = layout.area_mut(id.default_area());
            area.panels.push(id);
            area.active.get_or_insert(id);
        }
        layout.primary.visible = !layout.primary.panels.is_empty();
        layout
    }

    /// Adds registered panels missing from a saved layout (new in this
    /// version) to their default areas, and drops unknown ones.
    pub fn ensure_registered(&mut self, registered: &[PanelId]) {
        for a in [PanelArea::Primary, PanelArea::Secondary, PanelArea::Bottom] {
            let s = self.area_mut(a);
            s.panels.retain(|p| registered.contains(p));
            if s.active.is_some_and(|p| !s.panels.contains(&p)) {
                s.active = s.panels.first().copied();
            }
        }
        for &id in registered {
            if self.area_of(id).is_none() {
                let area = self.area_mut(id.default_area());
                area.panels.push(id);
                area.active.get_or_insert(id);
            }
        }
    }

    pub fn area(&self, area: PanelArea) -> &AreaState {
        match area {
            PanelArea::Primary => &self.primary,
            PanelArea::Secondary => &self.secondary,
            PanelArea::Bottom => &self.bottom,
        }
    }

    pub fn area_mut(&mut self, area: PanelArea) -> &mut AreaState {
        match area {
            PanelArea::Primary => &mut self.primary,
            PanelArea::Secondary => &mut self.secondary,
            PanelArea::Bottom => &mut self.bottom,
        }
    }

    pub fn area_of(&self, panel: PanelId) -> Option<PanelArea> {
        [PanelArea::Primary, PanelArea::Secondary, PanelArea::Bottom]
            .into_iter()
            .find(|a| self.area(*a).panels.contains(&panel))
    }

    /// Whether `panel` is currently on screen.
    pub fn is_shown(&self, panel: PanelId) -> bool {
        self.area_of(panel).is_some_and(|a| {
            let s = self.area(a);
            s.visible && s.active == Some(panel)
        })
    }

    /// Shows `panel` in its area (making it the active one).
    pub fn show(&mut self, panel: PanelId) -> bool {
        let Some(area) = self.area_of(panel) else {
            return false;
        };
        let s = self.area_mut(area);
        s.active = Some(panel);
        s.visible = true;
        true
    }

    /// View → Panels toggle: hides the area if `panel` is showing, else shows it.
    pub fn toggle(&mut self, panel: PanelId) {
        if self.is_shown(panel) {
            if let Some(area) = self.area_of(panel) {
                self.area_mut(area).visible = false;
            }
        } else {
            self.show(panel);
        }
    }

    /// Activity bar click: clicking the active panel collapses the sidebar.
    pub fn activity_click(&mut self, panel: PanelId) {
        self.toggle(panel);
    }

    pub fn toggle_area(&mut self, area: PanelArea) {
        let s = self.area_mut(area);
        if s.panels.is_empty() {
            s.visible = false;
            return;
        }
        s.visible = !s.visible;
        if s.active.is_none() {
            s.active = s.panels.first().copied();
        }
    }

    /// Moves a panel to another area (if allowed) and shows it there. The
    /// source area hides when it becomes empty.
    pub fn move_panel(&mut self, panel: PanelId, to: PanelArea) -> bool {
        if !panel.allowed_areas().contains(&to) {
            return false;
        }
        let Some(from) = self.area_of(panel) else {
            return false;
        };
        if from == to {
            return self.show(panel);
        }
        let src = self.area_mut(from);
        src.panels.retain(|p| *p != panel);
        if src.active == Some(panel) {
            src.active = src.panels.first().copied();
        }
        if src.panels.is_empty() {
            src.visible = false;
            src.maximized = false;
        }
        self.area_mut(to).panels.push(panel);
        self.show(panel)
    }

    pub fn set_primary_side(&mut self, side: Side) {
        self.primary_side = side;
    }

    /// Screen side of the secondary sidebar (opposite the primary).
    pub fn secondary_side(&self) -> Side {
        match self.primary_side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    pub fn set_size(&mut self, area: PanelArea, size: f32) {
        self.area_mut(area).size = size.max(area.min_size());
    }

    /// Hides every area (Maximize Editor Group, Zen Mode); restore by
    /// assigning back the clone taken before.
    pub fn hide_all(&mut self) {
        for a in [PanelArea::Primary, PanelArea::Secondary, PanelArea::Bottom] {
            self.area_mut(a).visible = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[PanelId] = &[
        PanelId::Explorer,
        PanelId::Search,
        PanelId::Outline,
        PanelId::Output,
    ];

    // spec 17: "Bottom panel hidden by default"
    #[test]
    fn fresh_layout_shows_only_primary() {
        let l = PanelLayout::new(ALL);
        assert!(l.primary.visible);
        assert_eq!(l.primary.active, Some(PanelId::Explorer));
        assert!(!l.secondary.visible);
        assert!(!l.bottom.visible);
    }

    // spec 17: "Move Outline to the bottom panel"
    #[test]
    fn moving_outline_to_bottom_hides_empty_secondary() {
        let mut l = PanelLayout::new(ALL);
        l.show(PanelId::Outline);
        assert!(l.move_panel(PanelId::Outline, PanelArea::Bottom));
        assert_eq!(l.area_of(PanelId::Outline), Some(PanelArea::Bottom));
        assert!(l.is_shown(PanelId::Outline));
        assert!(!l.secondary.visible);
        assert!(
            !l.move_panel(PanelId::Output, PanelArea::Primary),
            "not an allowed area"
        );
    }

    // spec 17: "Clicking the active activity icon collapses the sidebar"
    #[test]
    fn activity_click_on_active_panel_collapses() {
        let mut l = PanelLayout::new(ALL);
        l.activity_click(PanelId::Explorer);
        assert!(!l.primary.visible);
        l.activity_click(PanelId::Search);
        assert!(l.is_shown(PanelId::Search));
    }

    // spec 17: "Primary sidebar on the right"
    #[test]
    fn primary_side_switches_secondary_side() {
        let mut l = PanelLayout::new(ALL);
        l.set_primary_side(Side::Right);
        assert_eq!(l.secondary_side(), Side::Left);
    }

    #[test]
    fn sizes_clamp_to_minimum() {
        let mut l = PanelLayout::new(ALL);
        l.set_size(PanelArea::Bottom, 20.0);
        assert_eq!(l.bottom.size, 100.0);
    }

    #[test]
    fn hide_all_and_restore() {
        let mut l = PanelLayout::new(ALL);
        l.show(PanelId::Output);
        let saved = l.clone();
        l.hide_all();
        assert!(!l.primary.visible && !l.bottom.visible);
        l = saved;
        assert!(l.bottom.visible);
    }
}
