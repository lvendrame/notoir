//! Explorer panel (docs/specs/07 §Explorer Tree): header toolbar, filter box,
//! virtualized tree rows, inline name editor, context menus, keyboard
//! navigation and drag and drop (tree moves, OS drops, drags into editors).

use std::path::{Path, PathBuf};

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::explorer::{NodeKind, Row, filter_rows};
use notoir_core::when::ContextSnapshot;
use serde_json::json;

use super::{Actions, Kit, icon_button_sized};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::menus::{self, MenuItem};
use crate::t;
use crate::theme::Tok;
use crate::workspace::EditKind;

pub const ROW_H: f32 = 22.0;
const INDENT: f32 = 12.0;

/// Explorer state that lives with the UI (drag, focus).
#[derive(Default)]
pub struct ExplorerUi {
    /// A tree entry being dragged (into a folder or onto an editor group).
    pub drag: Option<PathBuf>,
    /// Folder under the pointer while dragging (drop target highlight).
    drop_dir: Option<PathBuf>,
}

pub fn tree_id() -> egui::Id {
    egui::Id::new("explorer-tree")
}

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

/// Header buttons (New File, New Folder, Refresh, Collapse All), shown while
/// the pointer is over the panel.
pub fn toolbar(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    if core.workspace.is_none() {
        return;
    }
    for (icon, id, key) in [
        (
            Icon::ChevronsDownUp,
            "explorer.collapseAll",
            "commands:explorer.collapseAll",
        ),
        (
            Icon::RefreshCw,
            "explorer.refresh",
            "commands:explorer.refresh",
        ),
        (
            Icon::FolderPlus,
            "explorer.newFolder",
            "commands:explorer.newFolder",
        ),
        (
            Icon::FilePlus,
            "explorer.newFile",
            "commands:explorer.newFile",
        ),
    ] {
        if icon_button_sized(ui, kit, icon, &t!(key), false, true, 24.0).clicked() {
            cmd(actions, id, json!({}));
        }
    }
}

/// Items for the panel's ⋯ menu.
pub fn more_menu_items() -> Vec<MenuItem> {
    let check = |id: &'static str, key: &str| MenuItem::Command {
        id,
        args: json!({}),
        label: None,
        checked: Some(key.to_owned()),
    };
    let sort = |value: &'static str, label: &str| MenuItem::Command {
        id: "explorer.sortOrder",
        args: json!({ "value": value }),
        label: Some(label.to_owned()),
        checked: Some(format!("explorerSort.{value}")),
    };
    vec![
        check("explorer.toggleHidden", "explorerShowHidden"),
        check("explorer.toggleGitIgnored", "explorerShowGitIgnored"),
        MenuItem::Submenu {
            label: "explorer:sortOrder",
            items: vec![
                sort("default", "explorer:sort.default"),
                sort("mixed", "explorer:sort.mixed"),
                sort("modified", "explorer:sort.modified"),
            ],
        },
    ]
}

fn context_items(core: &AppCore, row: &Row) -> Vec<MenuItem> {
    let args = json!({ "path": row.path.to_string_lossy() });
    let item = |id: &'static str| MenuItem::Command {
        id,
        args: args.clone(),
        label: None,
        checked: None,
    };
    let reveal = MenuItem::Command {
        id: "explorer.reveal",
        args: args.clone(),
        label: Some(notoir_platform::reveal_label_key().to_owned()),
        checked: None,
    };
    let is_root = core.workspace.as_ref().is_some_and(|w| w.root == row.path);
    match row.kind {
        NodeKind::File => vec![
            item("explorer.open"),
            item("explorer.openToSide"),
            MenuItem::Separator,
            item("explorer.rename"),
            item("explorer.duplicate"),
            item("explorer.delete"),
            MenuItem::Separator,
            item("explorer.copy"),
            item("explorer.copyPath"),
            item("explorer.copyRelativePath"),
            MenuItem::Separator,
            item("compare.with"),
            item("compare.select"),
            item("compare.withSelected"),
            MenuItem::Separator,
            reveal,
            MenuItem::Separator,
            // Only for files in a Git repository.
            if core.git_rel(&row.path).is_some() {
                MenuItem::Submenu {
                    label: "menus:sub.git",
                    items: vec![
                        item("git.compareWithHead"),
                        item("git.fileHistory"),
                        item("git.compareWithRevision"),
                        item("git.openFileAtRevision"),
                    ],
                }
            } else {
                MenuItem::Separator
            },
        ],
        NodeKind::Dir => {
            let mut v = vec![item("explorer.newFile"), item("explorer.newFolder")];
            if core
                .workspace
                .as_ref()
                .is_some_and(|w| w.clipboard.is_some())
            {
                v.push(item("explorer.paste"));
            }
            v.push(MenuItem::Separator);
            if !is_root {
                v.extend([
                    item("explorer.rename"),
                    item("explorer.duplicate"),
                    item("explorer.delete"),
                    MenuItem::Separator,
                ]);
            }
            v.extend([
                item("explorer.findInFolder"),
                MenuItem::Separator,
                item("explorer.copyPath"),
                item("explorer.copyRelativePath"),
                reveal,
            ]);
            if is_root {
                v.extend([
                    MenuItem::Separator,
                    item("explorer.refresh"),
                    item("explorer.collapseAll"),
                ]);
            }
            v
        }
    }
}

enum Line {
    Node(Row),
    /// The inline editor for a new entry, as the first child of its folder.
    NewEntry {
        depth: usize,
        folder: bool,
    },
}

fn file_icon(name: &str) -> Icon {
    let lang = notoir_editor::language::detect(Some(name), "", &[]);
    crate::shell::tabs::language_icon(lang.as_str())
}

/// The panel body.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    context: &ContextSnapshot,
    state: &mut ExplorerUi,
    actions: &mut Actions,
) {
    let Some(ws) = core.workspace.as_ref() else {
        no_folder(ui, kit, actions);
        return;
    };
    let opts = ws.row_options(&core.settings);
    let dim_ignored = core.settings.bool("explorer.dimGitIgnored", None);

    // Filter box.
    if ws.filter_open {
        let mut text = ws.filter.clone();
        let focus = ws.filter_focus;
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(egui::Id::new("explorer-filter"))
                .hint_text(t!("explorer:filterHint"))
                .desired_width(f32::INFINITY),
        );
        if focus {
            resp.request_focus();
        }
        let esc = resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape));
        let down = resp.has_focus() && ui.input(|i| i.key_pressed(Key::ArrowDown));
        let ws = core.workspace.as_mut().expect("checked");
        ws.filter_focus = false;
        ws.filter = text;
        if esc || (resp.lost_focus() && ws.filter.is_empty()) {
            ws.filter.clear();
            ws.filter_open = false;
            ui.memory_mut(|m| m.request_focus(tree_id()));
        } else if down {
            ui.memory_mut(|m| m.request_focus(tree_id()));
        }
        ui.add_space(4.0);
    }

    let ws = core.workspace.as_ref().expect("checked");
    let filtering = ws.filter_open && !ws.filter.trim().is_empty();
    let rows = if filtering {
        filter_rows(&ws.root, &ws.name, &ws.index, &ws.filter, &opts)
    } else {
        ws.tree.rows(&opts)
    };
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len() + 1);
    let new_in = match ws.edit.as_ref().map(|e| &e.kind) {
        Some(EditKind::NewFile { dir, .. }) => Some((dir.clone(), false)),
        Some(EditKind::NewFolder { dir }) => Some((dir.clone(), true)),
        _ => None,
    };
    for r in rows {
        let insert = new_in
            .as_ref()
            .filter(|(d, _)| *d == r.path)
            .map(|(_, f)| (r.depth + 1, *f));
        lines.push(Line::Node(r));
        if let Some((depth, folder)) = insert {
            lines.push(Line::NewEntry { depth, folder });
        }
    }
    let selected = ws.tree.selected.clone();
    let scroll_to = ws
        .scroll_to_selected
        .then(|| {
            lines
                .iter()
                .position(|l| matches!(l, Line::Node(r) if Some(&r.path) == selected.as_ref()))
        })
        .flatten();
    if let Some(ws) = core.workspace.as_mut() {
        ws.scroll_to_selected = false;
    }

    // Keyboard focus target for the whole tree.
    let area = ui.available_rect_before_wrap();
    let tree_resp = ui.interact(area, tree_id(), Sense::focusable_noninteractive());
    let has_focus = tree_resp.has_focus();
    if has_focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                tree_id(),
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            )
        });
    }
    core.explorer_focused = has_focus;

    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("explorer-scroll")
        .auto_shrink([false, false]);
    if let Some(i) = scroll_to {
        let viewport = ui.available_height();
        scroll = scroll.vertical_scroll_offset((i as f32 * ROW_H - viewport / 2.0).max(0.0));
    }
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let mut hovered_row: Option<Row> = None;
    let mut list_rect = Rect::NOTHING;
    let total = lines.len();
    let out = scroll.show_rows(ui, ROW_H, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for line in &lines[range] {
            match line {
                Line::Node(row) => {
                    let resp = node_row(
                        ui,
                        kit,
                        core,
                        row,
                        selected.as_ref() == Some(&row.path),
                        dim_ignored,
                        state,
                        actions,
                    );
                    if pointer.is_some_and(|p| resp.rect.contains(p)) {
                        hovered_row = Some(row.clone());
                    }
                    resp.context_menu(|ui| {
                        ui.set_min_width(240.0);
                        let items = menus::visible_items(&context_items(core, row), &core.commands);
                        super::chrome::menu_items(ui, kit, core, context, &items, actions);
                    });
                }
                Line::NewEntry { depth, folder } => {
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), ROW_H),
                        Sense::hover(),
                    );
                    let x = rect.left() + 6.0 + *depth as f32 * INDENT + 14.0;
                    let icon = if *folder { Icon::Folder } else { Icon::File };
                    let icon_rect = Rect::from_center_size(
                        Pos2::new(x + 7.0, rect.center().y),
                        Vec2::splat(14.0),
                    );
                    kit.icon(ui.ctx(), icon, 14.0, kit.color(Tok::TextSecondary))
                        .paint_at(ui, icon_rect);
                    let edit_rect = Rect::from_min_max(
                        Pos2::new(x + 20.0, rect.top() + 1.0),
                        Pos2::new(rect.right() - 4.0, rect.bottom() - 1.0),
                    );
                    inline_editor(ui, kit, core, edit_rect, None);
                }
            }
        }
    });
    list_rect = list_rect.union(out.inner_rect);
    if tree_resp.clicked() {
        tree_resp.request_focus();
    }

    // Empty-area context menu.
    let empty_area = Rect::from_min_max(
        Pos2::new(list_rect.left(), list_rect.top() + total as f32 * ROW_H),
        list_rect.max,
    );
    if empty_area.height() > 0.0 {
        let resp = ui.interact(empty_area, egui::Id::new("explorer-empty"), Sense::click());
        resp.context_menu(|ui| {
            let items = vec![
                MenuItem::Command {
                    id: "explorer.newFile",
                    args: json!({}),
                    label: None,
                    checked: None,
                },
                MenuItem::Command {
                    id: "explorer.newFolder",
                    args: json!({}),
                    label: None,
                    checked: None,
                },
                MenuItem::Separator,
                MenuItem::Command {
                    id: "explorer.refresh",
                    args: json!({}),
                    label: None,
                    checked: None,
                },
                MenuItem::Command {
                    id: "explorer.collapseAll",
                    args: json!({}),
                    label: None,
                    checked: None,
                },
            ];
            let items = menus::visible_items(&items, &core.commands);
            super::chrome::menu_items(ui, kit, core, context, &items, actions);
        });
        if resp.clicked() {
            if let Some(ws) = core.workspace.as_mut() {
                ws.tree.selected = None;
            }
            ui.memory_mut(|m| m.request_focus(tree_id()));
        }
    }

    if has_focus {
        keyboard(ui, core, &lines, actions);
    }
    drag_and_drop(
        ui,
        kit,
        core,
        state,
        list_rect,
        hovered_row.as_ref(),
        actions,
    );
}

fn no_folder(ui: &mut egui::Ui, kit: &mut Kit, actions: &mut Actions) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(t!("panels:explorer.noFolder"))
            .size(13.0)
            .color(kit.color(Tok::TextSecondary)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("panels:explorer.noFolderHint"))
            .size(12.0)
            .color(kit.color(Tok::TextTertiary)),
    );
    ui.add_space(10.0);
    if ui.button(t!("commands:workspace.open")).clicked() {
        cmd(actions, "workspace.open", json!({}));
    }
}

#[allow(clippy::too_many_arguments)]
fn node_row(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    row: &Row,
    selected: bool,
    dim_ignored: bool,
    state: &mut ExplorerUi,
    actions: &mut Actions,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_H),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    let is_drop_target = state.drag.is_some() && state.drop_dir.as_ref() == Some(&row.path);
    if is_drop_target {
        painter.rect_filled(rect, 2.0, kit.color(Tok::AccentSoft));
    } else if selected {
        let focused = ui.memory(|m| m.has_focus(tree_id()));
        painter.rect_filled(
            rect,
            2.0,
            if focused {
                kit.color(Tok::AccentSoft)
            } else {
                kit.color(Tok::BgPanelRaised)
            },
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, 2.0, kit.color(Tok::BgPanelRaised));
    }
    let mut x = rect.left() + 6.0 + row.depth as f32 * INDENT;
    let cy = rect.center().y;
    if row.kind == NodeKind::Dir {
        let chevron = if row.expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let r = Rect::from_center_size(Pos2::new(x + 6.0, cy), Vec2::splat(12.0));
        kit.icon(ui.ctx(), chevron, 12.0, kit.color(Tok::TextTertiary))
            .paint_at(ui, r);
    }
    x += 14.0;
    let icon = match row.kind {
        NodeKind::Dir if row.expanded => Icon::FolderOpen,
        NodeKind::Dir => Icon::Folder,
        NodeKind::File => file_icon(&row.name),
    };
    let dim = row.ignored && dim_ignored;
    let git = core.git_status_of(&row.path, row.kind == NodeKind::Dir);
    let text_color = if dim {
        kit.color(Tok::TextTertiary)
    } else if let Some(s) = git.filter(|_| row.kind == NodeKind::File) {
        kit.color(crate::theme::git_token(s))
    } else {
        kit.color(Tok::TextPrimary)
    };
    // Git: a letter badge for files, a dot in the strongest child colour
    // for folders (spec 07); colour is never the only signal.
    let mut badge_w = 0.0;
    if let Some(s) = git.filter(|_| !renaming_row(core, row)) {
        let c = kit.color(crate::theme::git_token(s));
        let bx = rect.right() - 14.0;
        if row.kind == NodeKind::File {
            painter.text(
                Pos2::new(bx, cy),
                egui::Align2::CENTER_CENTER,
                s.letter(),
                egui::FontId::monospace(11.5),
                c,
            );
        } else {
            painter.circle_filled(Pos2::new(bx, cy), 3.0, c);
            // Deleted files show in their folder as a count.
            let deleted = core
                .git_rel(&row.path)
                .map_or(0, |rel| core.git.status.deleted_in(&rel));
            if deleted > 0 {
                painter.text(
                    Pos2::new(bx - 8.0, cy),
                    egui::Align2::RIGHT_CENTER,
                    format!("D{deleted}"),
                    egui::FontId::monospace(10.5),
                    kit.color(Tok::GitDeleted),
                );
                badge_w = 44.0;
            }
        }
        badge_w = f32::max(badge_w, 24.0);
        let r = Rect::from_center_size(Pos2::new(bx, cy), Vec2::splat(14.0));
        let label = t!(
            "explorer:gitStatus",
            name = row.name.as_str(),
            status = t!(git_status_key(s))
        );
        let b = ui.interact(r, ui.id().with(("git-badge", &row.path)), Sense::hover());
        b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    }
    let icon_color = if dim {
        kit.color(Tok::TextTertiary)
    } else {
        kit.color(Tok::TextSecondary)
    };
    kit.icon(ui.ctx(), icon, 14.0, icon_color).paint_at(
        ui,
        Rect::from_center_size(Pos2::new(x + 7.0, cy), Vec2::splat(14.0)),
    );
    x += 20.0;
    let renaming = core
        .workspace
        .as_ref()
        .and_then(|w| w.edit.as_ref())
        .is_some_and(|e| {
            e.kind
                == EditKind::Rename {
                    path: row.path.clone(),
                }
        });
    if renaming {
        let edit_rect = Rect::from_min_max(
            Pos2::new(x - 2.0, rect.top() + 1.0),
            Pos2::new(rect.right() - 4.0, rect.bottom() - 1.0),
        );
        let stem = match row.kind {
            NodeKind::File => row
                .name
                .rfind('.')
                .filter(|i| *i > 0)
                .unwrap_or(row.name.len()),
            NodeKind::Dir => row.name.len(),
        };
        inline_editor(
            ui,
            kit,
            core,
            edit_rect,
            Some(row.name[..stem].chars().count()),
        );
    } else {
        let mut job = LayoutJob::default();
        let font = egui::FontId::proportional(13.0);
        for (i, ch) in row.name.chars().enumerate() {
            let hl = row.highlight.contains(&i);
            job.append(
                &ch.to_string(),
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: if hl {
                        kit.color(Tok::Accent)
                    } else {
                        text_color
                    },
                    ..Default::default()
                },
            );
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        // The name stops before the Git badge.
        let clip = Rect::from_min_max(rect.min, Pos2::new(rect.right() - badge_w, rect.bottom()));
        painter.with_clip_rect(clip).galley(
            Pos2::new(x, cy - galley.size().y / 2.0),
            galley,
            text_color,
        );
    }
    // The row label for accessibility and tests.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &row.name));

    if resp.clicked() && !renaming {
        ui.memory_mut(|m| m.request_focus(tree_id()));
        match row.kind {
            NodeKind::Dir => core.toggle_dir(&row.path),
            NodeKind::File => {
                if let Some(ws) = core.workspace.as_mut() {
                    ws.tree.selected = Some(row.path.clone());
                }
                cmd(
                    actions,
                    "explorer.open",
                    json!({
                        "path": row.path.to_string_lossy(),
                        "preserveFocus": true,
                        "preview": true,
                    }),
                );
            }
        }
    }
    if resp.double_clicked() && row.kind == NodeKind::File && !renaming {
        cmd(
            actions,
            "explorer.open",
            json!({ "path": row.path.to_string_lossy() }),
        );
    }
    if resp.secondary_clicked()
        && let Some(ws) = core.workspace.as_mut()
    {
        ws.tree.selected = Some(row.path.clone());
    }
    let is_root = core.workspace.as_ref().is_some_and(|w| w.root == row.path);
    if resp.drag_started() && !is_root && !renaming {
        state.drag = Some(row.path.clone());
    }
    resp
}

/// The inline name editor (new entry or rename) inside `rect`.
fn inline_editor(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    rect: Rect,
    select_chars: Option<usize>,
) {
    let Some(edit) = core.workspace.as_ref().and_then(|w| w.edit.clone()) else {
        return;
    };
    let id = egui::Id::new("explorer-inline-edit");
    let mut text = edit.text.clone();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let resp = child.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(rect.width())
            .margin(egui::Margin::symmetric(4, 1)),
    );
    if edit.focus {
        resp.request_focus();
        let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let end = select_chars.unwrap_or(text.chars().count());
        st.cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
        st.store(ui.ctx(), id);
        if let Some(e) = core.workspace.as_mut().and_then(|w| w.edit.as_mut()) {
            e.focus = false;
        }
    }
    if text != edit.text {
        core.set_edit_text(text);
    }
    if let Some(err) = core
        .workspace
        .as_ref()
        .and_then(|w| w.edit.as_ref())
        .and_then(|e| e.error.clone())
    {
        ui.painter().rect_stroke(
            resp.rect,
            2.0,
            Stroke::new(1.0, kit.color(Tok::Danger)),
            egui::StrokeKind::Outside,
        );
        let tip = egui::Area::new(egui::Id::new("explorer-inline-error"))
            .order(egui::Order::Tooltip)
            .fixed_pos(Pos2::new(resp.rect.left(), resp.rect.bottom() + 2.0));
        tip.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(kit.color(Tok::BgPanelRaised))
                .stroke(Stroke::new(1.0, kit.color(Tok::Danger)))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(err)
                            .size(12.0)
                            .color(kit.color(Tok::TextPrimary)),
                    );
                });
        });
    }
    if resp.lost_focus() {
        let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if esc {
            core.cancel_edit();
        } else if enter {
            core.commit_edit();
            if core.workspace.as_ref().is_some_and(|w| w.edit.is_some()) {
                // Validation failed: keep editing.
                resp.request_focus();
            } else {
                ui.memory_mut(|m| m.request_focus(tree_id()));
            }
        } else {
            // Clicking elsewhere commits a valid name and drops an invalid one.
            core.commit_edit();
            core.cancel_edit();
        }
    }
}

/// Arrow keys, Enter and type-to-filter while the tree has focus.
fn keyboard(ui: &mut egui::Ui, core: &mut AppCore, lines: &[Line], actions: &mut Actions) {
    let rows: Vec<&Row> = lines
        .iter()
        .filter_map(|l| if let Line::Node(r) = l { Some(r) } else { None })
        .collect();
    if rows.is_empty() || core.workspace.as_ref().is_some_and(|w| w.edit.is_some()) {
        return;
    }
    let (up, down, left, right, enter, typed) = ui.input(|i| {
        let typed: String = if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt {
            String::new()
        } else {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Text(t) = e {
                        Some(t.as_str())
                    } else {
                        None
                    }
                })
                .collect()
        };
        (
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
            i.key_pressed(Key::Enter),
            typed,
        )
    });
    let selected = core
        .workspace
        .as_ref()
        .and_then(|w| w.tree.selected.clone());
    let idx = selected
        .as_ref()
        .and_then(|s| rows.iter().position(|r| &r.path == s));
    let select = |core: &mut AppCore, p: &Path| {
        if let Some(ws) = core.workspace.as_mut() {
            ws.tree.selected = Some(p.to_path_buf());
            ws.scroll_to_selected = true;
        }
    };
    if down {
        let i = idx.map_or(0, |i| (i + 1).min(rows.len() - 1));
        select(core, &rows[i].path);
    } else if up {
        let i = idx.map_or(0, |i| i.saturating_sub(1));
        select(core, &rows[i].path);
    } else if let Some(i) = idx {
        let row = rows[i];
        if right && row.kind == NodeKind::Dir {
            if row.expanded {
                if let Some(next) = rows.get(i + 1).filter(|r| r.depth > row.depth) {
                    select(core, &next.path);
                }
            } else {
                core.expand_dir(&row.path);
            }
        } else if left {
            if row.kind == NodeKind::Dir && row.expanded && row.depth > 0 {
                core.toggle_dir(&row.path);
            } else if let Some(parent) = rows[..i].iter().rev().find(|r| r.depth < row.depth) {
                select(core, &parent.path);
            }
        } else if enter {
            match row.kind {
                NodeKind::Dir => core.toggle_dir(&row.path),
                NodeKind::File => cmd(
                    actions,
                    "explorer.open",
                    json!({ "path": row.path.to_string_lossy() }),
                ),
            }
        }
    }
    if !typed.trim().is_empty()
        && let Some(ws) = core.workspace.as_mut()
    {
        ws.filter_open = true;
        ws.filter_focus = true;
        ws.filter.push_str(&typed);
    }
}

/// Tree drags (move/copy into folders), OS file drops into the tree.
fn drag_and_drop(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    state: &mut ExplorerUi,
    list_rect: Rect,
    hovered: Option<&Row>,
    actions: &mut Actions,
) {
    let target_dir = |row: Option<&Row>| -> Option<PathBuf> {
        let ws = core.workspace.as_ref()?;
        Some(match row {
            Some(r) if r.kind == NodeKind::Dir => r.path.clone(),
            Some(r) => r.path.parent().map_or(ws.root.clone(), Path::to_path_buf),
            None => ws.root.clone(),
        })
    };
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let inside = pointer.is_some_and(|p| list_rect.contains(p));

    // OS files dropped onto the tree are copied into the folder under the pointer.
    let dropped: Vec<PathBuf> = ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect()
    });
    if !dropped.is_empty() && inside {
        if let Some(dir) = target_dir(hovered) {
            let srcs: Vec<String> = dropped
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            cmd(
                actions,
                "explorer.drop",
                json!({ "srcs": srcs, "dir": dir.to_string_lossy(), "copy": true }),
            );
        }
        ui.ctx().input_mut(|i| i.raw.dropped_files.clear());
    }

    let Some(src) = state.drag.clone() else {
        state.drop_dir = None;
        return;
    };
    let (down, esc, alt) = ui.input(|i| (i.pointer.primary_down(), i.key_pressed(Key::Escape), i.modifiers.alt || i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: false, modifiers, .. } if modifiers.alt))));
    if esc {
        state.drag = None;
        state.drop_dir = None;
        return;
    }
    state.drop_dir = if inside { target_dir(hovered) } else { None };
    if down {
        if let Some(p) = pointer {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("explorer-drag"),
            ));
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            painter.text(
                p + Vec2::new(14.0, 8.0),
                egui::Align2::LEFT_TOP,
                name,
                egui::FontId::proportional(12.0),
                kit.color(Tok::TextSecondary),
            );
        }
        return;
    }
    // Released inside the tree: move/copy into the folder. Outside, the
    // editor area may take it (layout.rs); the app clears it after the frame.
    if inside {
        if let Some(dir) = state.drop_dir.take() {
            cmd(
                actions,
                "explorer.drop",
                json!({ "srcs": [src.to_string_lossy()], "dir": dir.to_string_lossy(), "copy": alt }),
            );
        }
        state.drag = None;
    }
}

fn renaming_row(core: &AppCore, row: &Row) -> bool {
    core.workspace
        .as_ref()
        .and_then(|w| w.edit.as_ref())
        .is_some_and(|e| {
            e.kind
                == EditKind::Rename {
                    path: row.path.clone(),
                }
        })
}

/// i18n key of a Git status name.
pub fn git_status_key(s: notoir_git::FileStatus) -> &'static str {
    use notoir_git::FileStatus as F;
    match s {
        F::Modified => "explorer:git.modified",
        F::Added => "explorer:git.added",
        F::Untracked => "explorer:git.untracked",
        F::Deleted => "explorer:git.deleted",
        F::Conflicted => "explorer:git.conflicted",
        F::Renamed => "explorer:git.renamed",
        F::Ignored => "explorer:git.ignored",
    }
}
