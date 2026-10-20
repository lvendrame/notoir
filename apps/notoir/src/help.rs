//! Help › Documentation and F1 (docs/specs/18): links into the published user
//! guide (`docs/guide`), picked for what the user is working on.

use notoir_core::document::DocumentOrigin;
use notoir_core::panels::PanelId;
use notoir_core::settings::SettingCategory;
use notoir_core::view::ViewKind;

use crate::core::AppCore;

/// The published user guide.
pub const GUIDE_URL: &str = "https://lvendrame.github.io/notoir/";

const WORKSPACES: &str = "workspaces.html";
const SETTINGS: &str = "settings.html";
const KEYBINDINGS: &str = "keybindings.html";
const FIRST_STEPS: &str = "first-steps.html";
const COMPARE: &str = "compare.html";
const PLUGINS: &str = "plugins.html";
const MARKDOWN: &str = "markdown.html";
const JSON: &str = "json.html";
const FIND: &str = "find-and-search.html";
const FILES: &str = "files.html";
const EDITING: &str = "editing.html";
const GIT: &str = "git.html";
const NAVIGATION: &str = "navigation.html";
const NOTES: &str = "notes.html";
const MCP: &str = "mcp.html";
pub const SHORTCUTS: &str = "reference/shortcuts.html";

/// Every page a link can point at (checked against `docs/guide/src`).
pub const PAGES: &[&str] = &[
    WORKSPACES,
    SETTINGS,
    KEYBINDINGS,
    FIRST_STEPS,
    COMPARE,
    PLUGINS,
    MARKDOWN,
    JSON,
    FIND,
    FILES,
    EDITING,
    GIT,
    NAVIGATION,
    NOTES,
    MCP,
    SHORTCUTS,
];

/// The guide URL of `page` (the guide's start page for `None`).
pub fn url(page: Option<&str>) -> String {
    format!("{GUIDE_URL}{}", page.unwrap_or(""))
}

/// The guide page for what the user is working on: the focused Explorer, the
/// active tab, then the visible sidebar panel. `None` when nothing matches.
pub fn topic(core: &AppCore) -> Option<&'static str> {
    if core.explorer_focused {
        return Some(WORKSPACES);
    }
    let wb = &core.workbench;
    if let Some(view) = wb.active_view().and_then(|v| wb.view(v)) {
        let page = match view.kind {
            ViewKind::Settings if core.settings_ui.category == Some(SettingCategory::Keyboard) => {
                Some(KEYBINDINGS)
            }
            ViewKind::Settings => Some(SETTINGS),
            ViewKind::Welcome => Some(FIRST_STEPS),
            ViewKind::Diff => Some(COMPARE),
            ViewKind::PluginManager => Some(PLUGINS),
            ViewKind::MarkdownPreview | ViewKind::MarkdownSplit => Some(MARKDOWN),
            ViewKind::JsonTree | ViewKind::JsonSplit => Some(JSON),
            ViewKind::Text => None,
        };
        if page.is_some() {
            return page;
        }
    }
    if let Some(view) = wb.active_editor().and_then(|v| wb.view(v))
        && let Some(doc) = wb.document(view.document_id)
    {
        if core.find.widgets.get(&view.id).is_some_and(|w| w.open) {
            return Some(FIND);
        }
        match doc.language.as_str() {
            "markdown" => return Some(MARKDOWN),
            "json" | "jsonc" | "jsonl" => return Some(JSON),
            _ => {}
        }
        if core.editor_focused {
            return Some(match doc.origin {
                DocumentOrigin::Scratchpad { .. } | DocumentOrigin::Untitled { .. } => FILES,
                _ => EDITING,
            });
        }
    }
    let primary = &core.panels.primary;
    if !primary.visible {
        return None;
    }
    match primary.active? {
        PanelId::Explorer => Some(WORKSPACES),
        PanelId::Search => Some(FIND),
        PanelId::SourceControl | PanelId::GitOutput => Some(GIT),
        PanelId::Outline => Some(NAVIGATION),
        PanelId::Plugins | PanelId::PluginOutput => Some(PLUGINS),
        PanelId::Backlinks | PanelId::Tags => Some(NOTES),
        PanelId::McpActivity => Some(MCP),
        PanelId::MarkdownPreview => Some(MARKDOWN),
        PanelId::JsonTree => Some(JSON),
        PanelId::Output | PanelId::Problems => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_exists_in_the_guide() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide/src");
        for page in PAGES {
            let md = src.join(page.replace(".html", ".md"));
            assert!(md.is_file(), "{} is missing", md.display());
        }
    }

    #[test]
    fn url_joins_the_guide_root() {
        assert_eq!(url(None), GUIDE_URL);
        assert_eq!(
            url(Some(MARKDOWN)),
            "https://lvendrame.github.io/notoir/markdown.html"
        );
    }
}
