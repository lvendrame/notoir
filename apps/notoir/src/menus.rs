//! Main menu as data (docs/specs/18-menus-toolbar-and-context-menus.md).
//!
//! Every item is a command. The structure lists the full product menu; items
//! whose command is not registered yet are hidden, and a menu left without
//! items shows a single disabled "Coming soon" entry, so the top-level menu
//! set is stable from the first release.

use notoir_core::command::CommandRegistry;
use notoir_core::keymap::Platform;
use serde_json::{Value, json};

use crate::core::{AppCore, REGISTERED_PANELS, panel_key};

#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    Command {
        id: &'static str,
        args: Value,
        /// Label key overriding the command title (e.g. "Toolbar" in
        /// View → Appearance instead of "Toggle Toolbar").
        label: Option<String>,
        /// Context key rendered as a check mark.
        checked: Option<String>,
    },
    Submenu {
        label: &'static str,
        items: Vec<MenuItem>,
    },
    /// A command with a literal (user data) label, e.g. a recent file path.
    Entry {
        id: &'static str,
        args: Value,
        text: String,
    },
    /// Replaced at render time by [`expand`].
    Dynamic(DynamicMenu),
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicMenu {
    RecentWorkspaces,
    RecentFiles,
    KeptScratchpads,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub label: &'static str,
    pub items: Vec<MenuItem>,
}

fn cmd(id: &'static str) -> MenuItem {
    MenuItem::Command {
        id,
        args: Value::Null,
        label: None,
        checked: None,
    }
}

fn toggle(id: &'static str, label: &str, checked: &str) -> MenuItem {
    MenuItem::Command {
        id,
        args: Value::Null,
        label: Some(label.to_owned()),
        checked: Some(checked.to_owned()),
    }
}

fn sub(label: &'static str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem::Submenu { label, items }
}

const SEP: MenuItem = MenuItem::Separator;

/// The full menu bar for `platform`.
pub fn menu_bar(platform: Platform) -> Vec<Menu> {
    let mac = platform == Platform::Mac;
    let mut menus = Vec::new();
    if mac {
        menus.push(Menu {
            label: "menus:top.app",
            items: vec![
                cmd("help.about"),
                SEP,
                cmd("app.settings"),
                SEP,
                cmd("app.quit"),
            ],
        });
    }

    let mut file = vec![
        cmd("file.newUntitled"),
        cmd("file.newScratchpad"),
        cmd("file.newWindow"),
        SEP,
        cmd("file.open"),
        cmd("workspace.open"),
        sub(
            "menus:sub.openRecent",
            vec![
                MenuItem::Dynamic(DynamicMenu::RecentWorkspaces),
                SEP,
                MenuItem::Dynamic(DynamicMenu::RecentFiles),
                SEP,
                sub(
                    "menus:sub.scratchpads",
                    vec![MenuItem::Dynamic(DynamicMenu::KeptScratchpads)],
                ),
                SEP,
                cmd("file.clearRecent"),
            ],
        ),
        SEP,
        cmd("file.save"),
        cmd("file.saveAs"),
        cmd("file.saveAll"),
        cmd("file.autoSave"),
        cmd("file.revert"),
        SEP,
        cmd("file.close"),
        cmd("file.closeOthers"),
        cmd("file.closeAll"),
        cmd("workspace.close"),
        SEP,
        cmd("file.properties"),
    ];
    if !mac {
        file.extend([SEP, cmd("app.quit")]);
    }
    menus.push(Menu {
        label: "menus:top.file",
        items: file,
    });

    menus.push(Menu {
        label: "menus:top.edit",
        items: vec![
            cmd("edit.undo"),
            cmd("edit.redo"),
            SEP,
            cmd("edit.cut"),
            cmd("edit.copy"),
            cmd("edit.paste"),
            cmd("edit.pastePlain"),
            SEP,
            cmd("edit.find"),
            cmd("edit.replace"),
            cmd("edit.findNext"),
            cmd("edit.findPrevious"),
            cmd("search.workspace"),
            SEP,
            cmd("edit.selectAll"),
            SEP,
            cmd("edit.duplicateLine"),
            cmd("edit.deleteLine"),
            cmd("edit.moveLineUp"),
            cmd("edit.moveLineDown"),
            cmd("edit.sortLines"),
            cmd("edit.trimTrailingWhitespace"),
            sub(
                "menus:sub.convertCase",
                vec![
                    cmd("edit.transformToUppercase"),
                    cmd("edit.transformToLowercase"),
                    cmd("edit.transformToTitleCase"),
                ],
            ),
            SEP,
            cmd("edit.toggleReadOnly"),
        ],
    });

    menus.push(Menu {
        label: "menus:top.selection",
        items: vec![
            cmd("edit.selectAll"),
            cmd("selection.expand"),
            cmd("selection.shrink"),
            SEP,
            cmd("selection.addCursorAbove"),
            cmd("selection.addCursorBelow"),
            cmd("selection.selectNextOccurrence"),
            cmd("selection.selectAllOccurrences"),
            cmd("selection.columnMode"),
        ],
    });

    let panels: Vec<MenuItem> = REGISTERED_PANELS
        .iter()
        .map(|p| MenuItem::Command {
            id: "view.togglePanel",
            args: json!({ "panel": panel_key(*p) }),
            label: Some(p.title_key().to_owned()),
            checked: Some(format!("panel.{}.visible", panel_key(*p))),
        })
        .collect();
    menus.push(Menu {
        label: "menus:top.view",
        items: vec![
            cmd("workbench.commandPalette"),
            cmd("navigate.quickOpen"),
            SEP,
            sub("menus:sub.panels", panels),
            sub(
                "menus:sub.appearance",
                vec![
                    toggle(
                        "view.togglePrimarySidebar",
                        "menus:item.primarySidebar",
                        "primarySidebarVisible",
                    ),
                    toggle(
                        "view.toggleSecondarySidebar",
                        "menus:item.secondarySidebar",
                        "secondarySidebarVisible",
                    ),
                    toggle(
                        "view.toggleBottomPanel",
                        "menus:item.bottomPanel",
                        "bottomPanelVisible",
                    ),
                    toggle(
                        "view.toggleActivityBar",
                        "menus:item.activityBar",
                        "activityBarVisible",
                    ),
                    toggle(
                        "view.togglePrimarySidebarPosition",
                        "menus:item.primarySidebarRight",
                        "primarySidebarRight",
                    ),
                    SEP,
                    toggle("view.toggleToolbar", "menus:item.toolbar", "toolbarVisible"),
                    toggle(
                        "view.toggleStatusBar",
                        "menus:item.statusBar",
                        "statusBarVisible",
                    ),
                    cmd("view.toggleBreadcrumbs"),
                    toggle(
                        "view.toggleLineNumbers",
                        "menus:item.lineNumbers",
                        "lineNumbersVisible",
                    ),
                    cmd("view.toggleMinimap"),
                    toggle(
                        "view.toggleWhitespace",
                        "menus:item.whitespace",
                        "whitespaceVisible",
                    ),
                    toggle(
                        "view.toggleCurrentLineHighlight",
                        "menus:item.currentLineHighlight",
                        "currentLineHighlight",
                    ),
                ],
            ),
            sub(
                "menus:sub.layout",
                vec![
                    cmd("view.splitLeft"),
                    cmd("view.splitRight"),
                    cmd("view.splitUp"),
                    cmd("view.splitDown"),
                    cmd("view.singleEditor"),
                    cmd("workbench.maximizeGroup"),
                ],
            ),
            SEP,
            toggle("view.toggleWordWrap", "menus:item.wordWrap", "wordWrap"),
            cmd("fold.fold"),
            cmd("fold.unfold"),
            cmd("fold.foldAll"),
            cmd("fold.unfoldAll"),
            SEP,
            cmd("view.zoomIn"),
            cmd("view.zoomOut"),
            cmd("view.zoomReset"),
            SEP,
            toggle(
                "view.toggleFullScreen",
                "commands:view.toggleFullScreen",
                "fullScreen",
            ),
            cmd("view.zenMode"),
        ],
    });

    menus.push(Menu {
        label: "menus:top.navigate",
        items: vec![
            cmd("navigate.quickOpen"),
            cmd("navigate.goToLine"),
            cmd("navigate.goToSymbol"),
            SEP,
            cmd("navigate.nextTab"),
            cmd("navigate.previousTab"),
            cmd("workbench.focusNextGroup"),
            cmd("workbench.focusPreviousGroup"),
            SEP,
            cmd("navigate.back"),
            cmd("navigate.forward"),
            SEP,
            cmd("navigate.nextChange"),
            cmd("navigate.previousChange"),
            cmd("bookmarks.next"),
            cmd("bookmarks.previous"),
            cmd("navigate.nextSearchResult"),
            cmd("navigate.previousSearchResult"),
        ],
    });

    menus.push(Menu {
        label: "menus:top.git",
        items: vec![
            cmd("git.showSourceControl"),
            cmd("git.status"),
            SEP,
            cmd("git.commit"),
            cmd("git.commitAll"),
            SEP,
            cmd("git.pull"),
            cmd("git.push"),
            cmd("git.sync"),
            cmd("git.fetch"),
            SEP,
            sub(
                "menus:sub.branches",
                vec![cmd("git.switchBranch"), cmd("git.createBranch")],
            ),
            SEP,
            cmd("git.viewFileChanges"),
            cmd("git.compareWithHead"),
            cmd("git.fileHistory"),
            SEP,
            cmd("git.init"),
            cmd("git.clone"),
        ],
    });

    let mut tools = vec![
        cmd("compare.open"),
        cmd("editor.formatDocument"),
        SEP,
        sub(
            "menus:sub.mcp",
            vec![
                cmd("mcp.startServer"),
                cmd("mcp.stopServer"),
                cmd("mcp.connections"),
                cmd("mcp.permissions"),
                cmd("mcp.activity"),
                cmd("mcp.settings"),
            ],
        ),
        SEP,
        cmd("app.reloadWindow"),
        cmd("plugins.reload"),
        SEP,
    ];
    if !mac {
        tools.push(cmd("app.settings"));
    }
    tools.extend([
        cmd("app.keyboardShortcuts"),
        cmd("preferences.changeTheme"),
        cmd("preferences.changeLanguage"),
    ]);
    menus.push(Menu {
        label: "menus:top.tools",
        items: tools,
    });

    menus.push(Menu {
        label: "menus:top.plugins",
        items: vec![
            cmd("plugins.manager"),
            cmd("plugins.installed"),
            cmd("plugins.reload"),
            SEP,
            cmd("plugins.openFolder"),
            cmd("plugins.developerTools"),
        ],
    });

    let mut help = vec![
        cmd("help.welcome"),
        cmd("help.documentation"),
        cmd("help.keyboardReference"),
        SEP,
        cmd("help.reportIssue"),
        cmd("help.viewLogs"),
        cmd("help.recoverUnsaved"),
        SEP,
        cmd("app.checkForUpdates"),
    ];
    if !mac {
        help.extend([SEP, cmd("help.about")]);
    }
    menus.push(Menu {
        label: "menus:top.help",
        items: help,
    });
    menus
}

/// Replaces dynamic placeholders with entries from the app state.
pub fn expand(items: &[MenuItem], core: &AppCore) -> Vec<MenuItem> {
    let mut out = Vec::new();
    for item in items {
        match item {
            MenuItem::Dynamic(DynamicMenu::RecentFiles) => {
                for r in core
                    .files
                    .recent_files
                    .iter()
                    .take(crate::files::RECENT_SHOWN)
                {
                    // Pinned entries come first (state DB ordering).
                    out.push(MenuItem::Entry {
                        id: "file.openRecent",
                        args: json!({ "path": r.path }),
                        text: r.path.clone(),
                    });
                }
            }
            MenuItem::Dynamic(DynamicMenu::RecentWorkspaces) => {
                for r in core
                    .recent_workspaces()
                    .iter()
                    .take(crate::files::RECENT_SHOWN)
                {
                    let name = r.display_name.clone().unwrap_or_else(|| {
                        std::path::Path::new(&r.path)
                            .file_name()
                            .map_or_else(|| r.path.clone(), |n| n.to_string_lossy().into_owned())
                    });
                    out.push(MenuItem::Entry {
                        id: "workspace.openRecent",
                        args: json!({ "path": r.path }),
                        text: name,
                    });
                }
            }
            MenuItem::Dynamic(DynamicMenu::KeptScratchpads) => {
                for s in core.kept_scratchpads() {
                    out.push(MenuItem::Entry {
                        id: "file.openScratchpad",
                        args: json!({ "id": s.scratch_id }),
                        text: s.title,
                    });
                }
            }
            MenuItem::Submenu { label, items } => out.push(MenuItem::Submenu {
                label,
                items: expand(items, core),
            }),
            other => out.push(other.clone()),
        }
    }
    out
}

/// Items whose commands exist, with separators normalized (no leading,
/// trailing or doubled separators) and empty submenus removed.
pub fn visible_items(items: &[MenuItem], registry: &CommandRegistry<AppCore>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::new();
    for item in items {
        match item {
            MenuItem::Command { id, .. } | MenuItem::Entry { id, .. } => {
                if registry.get(&(*id).into()).is_some() {
                    out.push(item.clone());
                }
            }
            MenuItem::Dynamic(_) => {}
            MenuItem::Submenu { label, items } => {
                let inner = visible_items(items, registry);
                if !inner.is_empty() {
                    out.push(MenuItem::Submenu {
                        label,
                        items: inner,
                    });
                }
            }
            MenuItem::Separator => {
                if !matches!(out.last(), None | Some(MenuItem::Separator)) {
                    out.push(MenuItem::Separator);
                }
            }
        }
    }
    if matches!(out.last(), Some(MenuItem::Separator)) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(platform: Platform) -> CommandRegistry<AppCore> {
        let mut r = CommandRegistry::default();
        crate::commands::register_all(&mut r, platform);
        r
    }

    fn labels(platform: Platform) -> Vec<&'static str> {
        menu_bar(platform).iter().map(|m| m.label).collect()
    }

    // spec 18: "No MCP top-level menu"
    #[test]
    fn top_level_menus_are_fixed() {
        let common = [
            "menus:top.file",
            "menus:top.edit",
            "menus:top.selection",
            "menus:top.view",
            "menus:top.navigate",
            "menus:top.git",
            "menus:top.tools",
            "menus:top.plugins",
            "menus:top.help",
        ];
        assert_eq!(labels(Platform::Windows), common);
        assert_eq!(labels(Platform::Mac)[0], "menus:top.app");
        assert_eq!(labels(Platform::Mac)[1..], common);
    }

    fn contains(items: &[MenuItem], id: &str) -> bool {
        items.iter().any(|i| match i {
            MenuItem::Command { id: c, .. } => *c == id,
            MenuItem::Submenu { items, .. } => contains(items, id),
            MenuItem::Entry { id: c, .. } => *c == id,
            MenuItem::Dynamic(_) | MenuItem::Separator => false,
        })
    }

    #[test]
    fn quit_and_about_follow_platform_conventions() {
        let mac = menu_bar(Platform::Mac);
        assert!(contains(&mac[0].items, "app.quit") && contains(&mac[0].items, "help.about"));
        assert!(
            !contains(&mac[1].items, "app.quit"),
            "no Exit in the macOS File menu"
        );
        let win = menu_bar(Platform::Windows);
        assert!(contains(&win[0].items, "app.quit"));
        assert!(contains(&win.last().unwrap().items, "help.about"));
    }

    #[test]
    fn unregistered_items_are_hidden_and_separators_normalized() {
        let r = registry(Platform::Linux);
        for menu in menu_bar(Platform::Linux) {
            let items = visible_items(&menu.items, &r);
            assert!(
                !matches!(items.first(), Some(MenuItem::Separator)),
                "{}",
                menu.label
            );
            assert!(
                !matches!(items.last(), Some(MenuItem::Separator)),
                "{}",
                menu.label
            );
            assert!(items.windows(2).all(|w| !(w[0] == SEP && w[1] == SEP)));
        }
        let view = &menu_bar(Platform::Linux)[3];
        let items = visible_items(&view.items, &r);
        assert!(contains(&items, "view.toggleToolbar"));
        assert!(!contains(&items, "view.zenMode"));
    }
}
