//! Source Control panel (docs/specs/28 §Source Control Panel): commit message
//! (Mod+Enter commits), Commit with its menu, Staged Changes / Changes /
//! Merge Conflicts with hover actions and context menus, Sync Changes.
//! Rendering only: every action is a command.

use std::collections::HashSet;

use egui::{Align, Align2, Key, Layout, Modifiers, RichText, Sense, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_git::FileStatus;
use serde_json::json;

use super::{Actions, Kit, icon_button_sized};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::t;
use crate::theme::Tok;

const ROW_HEIGHT: f32 = 22.0;
/// Subtle ruler on commit message body lines.
const RULER_COLUMN: f32 = 72.0;

/// Collapsed sections ("staged", "changes", "conflicts").
#[derive(Default)]
pub struct ScmUi {
    collapsed: HashSet<&'static str>,
}

fn inv(id: &str, args: serde_json::Value) -> CommandInvocation {
    CommandInvocation::new(id.to_owned(), InvocationSource::Internal).with_args(args)
}

fn status_label(s: FileStatus) -> String {
    use FileStatus as F;
    t!(match s {
        F::Modified => "explorer:git.modified",
        F::Added => "explorer:git.added",
        F::Untracked => "explorer:git.untracked",
        F::Deleted => "explorer:git.deleted",
        F::Conflicted => "explorer:git.conflicted",
        F::Renamed => "explorer:git.renamed",
        F::Ignored => "explorer:git.ignored",
    })
}

/// Header buttons (shown while the panel is hovered): Commit, Refresh.
pub fn toolbar(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    if core.git.repo.is_none() {
        return;
    }
    if icon_button_sized(
        ui,
        kit,
        Icon::RefreshCw,
        &t!("commands:git.refresh"),
        false,
        true,
        24.0,
    )
    .clicked()
    {
        actions.push(inv("git.refresh", json!({})));
    }
    if icon_button_sized(
        ui,
        kit,
        Icon::Check,
        &t!("commands:git.commit"),
        false,
        !core.git_busy(),
        24.0,
    )
    .clicked()
    {
        actions.push(inv("git.commit", json!({})));
    }
}

/// The `⋯` menu of the panel.
pub fn more_menu_items() -> Vec<crate::menus::MenuItem> {
    use crate::menus::{MenuItem, cmd};
    let sep = || MenuItem::Separator;
    vec![
        cmd("git.pull"),
        cmd("git.pullRebase"),
        cmd("git.push"),
        cmd("git.fetch"),
        cmd("git.sync"),
        cmd("git.syncNotes"),
        sep(),
        cmd("git.commitAll"),
        cmd("git.commitAmend"),
        cmd("git.commitAndPush"),
        sep(),
        cmd("git.stageAll"),
        cmd("git.unstageAll"),
        cmd("git.discardAll"),
        sep(),
        cmd("git.switchBranch"),
        cmd("git.createBranch"),
        cmd("git.createBranchFrom"),
        sep(),
        cmd("git.showOutput"),
    ]
}

pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    scm: &mut ScmUi,
    actions: &mut Actions,
) {
    let muted = |kit: &Kit, text: String| {
        RichText::new(text)
            .size(12.0)
            .color(kit.color(Tok::TextTertiary))
    };
    if core.workspace.is_none() {
        ui.label(muted(kit, t!("git:scm.noWorkspace")));
        return;
    }
    if core.git.repo.is_none() {
        ui.label(muted(kit, t!("git:scm.noRepo")));
        ui.add_space(8.0);
        if ui.button(t!("commands:git.init")).clicked() {
            actions.push(inv("git.init", json!({})));
        }
        if ui.button(t!("commands:git.clone")).clicked() {
            actions.push(inv("git.clone", json!({})));
        }
        return;
    }
    if let Some(notice) = &core.git.notice {
        ui.label(
            RichText::new(t!(&notice.key))
                .size(12.0)
                .color(kit.color(Tok::Warning)),
        );
        ui.add_space(6.0);
    }
    message_box(ui, kit, core, actions);
    ui.add_space(6.0);
    commit_button(ui, kit, core, actions);
    ui.add_space(8.0);

    let status = core.git.status.clone();
    let workdir = core
        .git
        .repo
        .as_ref()
        .map(|r| r.workdir.clone())
        .unwrap_or_default();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(ui.available_height() - 36.0)
        .show(ui, |ui| {
            let staged: Vec<(String, FileStatus)> =
                status.staged.iter().map(|(k, s)| (k.clone(), *s)).collect();
            let changes: Vec<(String, FileStatus)> =
                status.changes().map(|(k, s)| (k.clone(), s)).collect();
            let conflicts: Vec<(String, FileStatus)> = status
                .conflicts()
                .map(|k| (k.clone(), FileStatus::Conflicted))
                .collect();
            if !conflicts.is_empty() {
                section(
                    ui,
                    kit,
                    scm,
                    actions,
                    Section::Conflicts,
                    &conflicts,
                    &workdir,
                );
            }
            section(ui, kit, scm, actions, Section::Staged, &staged, &workdir);
            section(ui, kit, scm, actions, Section::Changes, &changes, &workdir);
            if status.truncated {
                ui.label(muted(kit, t!("git:scm.truncated")));
            }
        });
    sync_button(ui, kit, core, actions);
}

/// The commit message: Mod+Enter commits; a ruler marks 72 columns.
fn message_box(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    let id = egui::Id::new("scm-message");
    let focused = ui.memory(|m| m.has_focus(id));
    if focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        actions.push(inv("git.commit", json!({})));
    }
    let hint = t!("git:scm.messageHint", keys = mod_enter(core));
    let font = crate::fonts::mono(12.0);
    let Some(draft) = core.git_message_mut() else {
        return;
    };
    let resp = ui.add(
        egui::TextEdit::multiline(draft)
            .id(id)
            .hint_text(hint.clone())
            .font(font.clone())
            .desired_rows(3)
            .desired_width(ui.available_width()),
    );
    let label = t!("git:scm.message");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'm'));
    let x = resp.rect.left() + 4.0 + char_w * RULER_COLUMN;
    if x < resp.rect.right() - 4.0 {
        ui.painter().vline(
            x,
            resp.rect.y_range().shrink(4.0),
            egui::Stroke::new(1.0, kit.color(Tok::Border)),
        );
    }
}

fn mod_enter(core: &AppCore) -> String {
    notoir_core::keymap::KeyChord::parse("Mod+Enter", core.platform)
        .map(|k| k.display(core.platform))
        .unwrap_or_default()
}

/// `[Commit] [⌄]`: Commit · Commit All · Commit & Push · Amend.
fn commit_button(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    let enabled = !core.git_busy();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 32.0).max(80.0);
        let text = RichText::new(t!("git:scm.commit")).color(kit.color(Tok::BgApp));
        let button = egui::Button::new(text)
            .fill(kit.color(Tok::Accent))
            .min_size(Vec2::new(width, 26.0));
        if ui.add_enabled(enabled, button).clicked() {
            actions.push(inv("git.commit", json!({})));
        }
        let more = icon_button_sized(
            ui,
            kit,
            Icon::ChevronDown,
            &t!("git:scm.commitMore"),
            false,
            enabled,
            26.0,
        );
        egui::Popup::menu(&more).show(|ui| {
            ui.set_min_width(200.0);
            for id in [
                "git.commit",
                "git.commitAll",
                "git.commitAndPush",
                "git.commitAmend",
            ] {
                if ui.button(t!(&format!("commands:{id}"))).clicked() {
                    actions.push(inv(id, json!({})));
                }
            }
        });
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Staged,
    Changes,
    Conflicts,
}

impl Section {
    fn key(self) -> &'static str {
        match self {
            Section::Staged => "staged",
            Section::Changes => "changes",
            Section::Conflicts => "conflicts",
        }
    }

    fn title(self) -> String {
        t!(match self {
            Section::Staged => "git:scm.staged",
            Section::Changes => "git:scm.changes",
            Section::Conflicts => "git:scm.conflicts",
        })
    }
}

fn section(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    scm: &mut ScmUi,
    actions: &mut Actions,
    which: Section,
    rows: &[(String, FileStatus)],
    workdir: &std::path::Path,
) {
    let collapsed = scm.collapsed.contains(which.key());
    let title = format!("{} ({})", which.title(), rows.len());
    let header = ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        let chevron = if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        ui.add(kit.icon(
            ui.ctx(),
            chevron,
            crate::theme::ICON_SM,
            kit.color(Tok::TextTertiary),
        ));
        let resp = ui.add(
            egui::Label::new(
                RichText::new(title.to_uppercase())
                    .font(crate::fonts::extrabold(11.0))
                    .color(kit.color(Tok::TextTertiary)),
            )
            .sense(Sense::click()),
        );
        if resp.clicked() {
            if collapsed {
                scm.collapsed.remove(which.key());
            } else {
                scm.collapsed.insert(which.key());
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let buttons: &[(Icon, &str)] = match which {
                Section::Staged => &[(Icon::Minus, "git.unstageAll")],
                Section::Changes => &[
                    (Icon::Plus, "git.stageAll"),
                    (Icon::Undo2, "git.discardAll"),
                ],
                Section::Conflicts => &[],
            };
            for (icon, id) in buttons {
                if !rows.is_empty()
                    && icon_button_sized(
                        ui,
                        kit,
                        *icon,
                        &t!(&format!("commands:{id}")),
                        false,
                        true,
                        22.0,
                    )
                    .clicked()
                {
                    actions.push(inv(id, json!({})));
                }
            }
        });
    });
    let _ = header;
    if collapsed {
        return;
    }
    for (rel, status) in rows {
        row(ui, kit, actions, which, rel, *status, workdir);
    }
}

fn row(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    actions: &mut Actions,
    which: Section,
    rel: &str,
    status: FileStatus,
    workdir: &std::path::Path,
) {
    let staged = which == Section::Staged;
    let abs = workdir.join(rel).to_string_lossy().into_owned();
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    let painter = ui.painter_at(rect);
    if hovered {
        painter.rect_filled(rect, crate::theme::RADIUS, kit.color(Tok::BgPanelRaised));
    }
    let color = kit.color(crate::theme::git_token(status));
    let (name, dir) = match rel.rsplit_once('/') {
        Some((d, n)) => (n, d),
        None => (rel, ""),
    };
    let name_rect = painter.text(
        rect.left_center() + Vec2::new(18.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(13.0),
        if status == FileStatus::Deleted {
            kit.color(Tok::TextTertiary)
        } else {
            color
        },
    );
    if !dir.is_empty() {
        painter.text(
            name_rect.right_center() + Vec2::new(6.0, 0.0),
            Align2::LEFT_CENTER,
            dir,
            egui::FontId::proportional(11.0),
            kit.color(Tok::TextTertiary),
        );
    }
    painter.text(
        rect.right_center() - Vec2::new(8.0, 0.0),
        Align2::RIGHT_CENTER,
        status.letter(),
        crate::fonts::mono(12.0),
        color,
    );
    let label = t!(
        "git:scm.row",
        name = rel,
        status = status_label(status),
        section = which.title()
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let paths = json!({ "paths": [rel] });
    if resp.double_clicked() {
        actions.push(inv("explorer.open", json!({ "path": abs })));
    } else if resp.clicked() {
        actions.push(inv(
            "git.openChange",
            json!({ "path": rel, "staged": staged }),
        ));
    }
    // Hover actions: Open File, Stage / Unstage, Discard.
    if hovered {
        let mut items: Vec<(Icon, String, CommandInvocation)> = vec![(
            Icon::File,
            t!("git:scm.openFile"),
            inv("explorer.open", json!({ "path": abs })),
        )];
        match which {
            Section::Staged => items.push((
                Icon::Minus,
                t!("commands:git.unstage"),
                inv("git.unstage", paths.clone()),
            )),
            Section::Changes => {
                items.push((
                    Icon::Undo2,
                    t!("commands:git.discard"),
                    inv("git.discard", paths.clone()),
                ));
                items.push((
                    Icon::Plus,
                    t!("commands:git.stage"),
                    inv("git.stage", paths.clone()),
                ));
            }
            Section::Conflicts => items.push((
                Icon::Plus,
                t!("commands:git.stage"),
                inv("git.stage", paths.clone()),
            )),
        }
        let mut x = rect.right() - 24.0;
        for (icon, tip, action) in items.into_iter().rev() {
            let r = egui::Rect::from_min_size(
                egui::pos2(x - 22.0, rect.center().y - 11.0),
                Vec2::splat(22.0),
            );
            x -= 23.0;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
            if icon_button_sized(&mut child, kit, icon, &tip, false, true, 22.0).clicked() {
                actions.push(action);
            }
        }
    }
    resp.context_menu(|ui| {
        ui.set_min_width(200.0);
        let mut item = |ui: &mut egui::Ui, label: String, action: CommandInvocation| {
            if ui.button(label).clicked() {
                actions.push(action);
                ui.close();
            }
        };
        item(
            ui,
            t!("git:scm.openFile"),
            inv("explorer.open", json!({ "path": abs })),
        );
        item(
            ui,
            t!("git:scm.openChanges"),
            inv("git.openChange", json!({ "path": rel, "staged": staged })),
        );
        ui.separator();
        if staged {
            item(
                ui,
                t!("commands:git.unstage"),
                inv("git.unstage", paths.clone()),
            );
        } else {
            item(
                ui,
                t!("commands:git.stage"),
                inv("git.stage", paths.clone()),
            );
        }
        if which == Section::Changes {
            item(
                ui,
                t!("commands:git.discard"),
                inv("git.discard", paths.clone()),
            );
        }
        ui.separator();
        item(
            ui,
            t!("git:scm.revealInExplorer"),
            inv("git.revealInExplorer", json!({ "path": abs })),
        );
        item(
            ui,
            t!("commands:explorer.copyPath"),
            inv("explorer.copyPath", json!({ "path": abs })),
        );
        item(
            ui,
            t!("commands:git.fileHistory"),
            inv("git.fileHistory", json!({ "path": abs })),
        );
    });
}

/// `Sync Changes ↑2 ↓1`, or Publish Branch without an upstream.
fn sync_button(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, actions: &mut Actions) {
    if core.git.remotes.is_empty() {
        return;
    }
    let head = &core.git.status.head;
    let (text, id) = if head.has_upstream {
        let mut s = t!("git:scm.sync");
        if head.ahead > 0 {
            s.push_str(&format!(" \u{2191}{}", head.ahead));
        }
        if head.behind > 0 {
            s.push_str(&format!(" \u{2193}{}", head.behind));
        }
        (s, "git.sync")
    } else if head.branch.is_some() {
        (t!("git:scm.publish"), "git.push")
    } else {
        return;
    };
    ui.add_space(6.0);
    let button = egui::Button::new(RichText::new(text).color(kit.color(Tok::TextPrimary)))
        .min_size(Vec2::new(ui.available_width(), 26.0));
    if ui.add_enabled(!core.git_busy(), button).clicked() {
        actions.push(inv(id, json!({})));
    }
}

/// Git Output: commands and git's own output, newest last.
pub fn output(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore) {
    if core.git.output.is_empty() {
        ui.label(
            RichText::new(t!("git:output.empty"))
                .size(12.0)
                .color(kit.color(Tok::TextTertiary)),
        );
        return;
    }
    let row_h = 16.0;
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, core.git.output.len(), |ui, range| {
            for line in core.git.output.range(range) {
                let color = if line.starts_with("> ") {
                    kit.color(Tok::TextPrimary)
                } else {
                    kit.color(Tok::TextSecondary)
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .font(crate::fonts::mono(12.0))
                            .color(color),
                    )
                    .selectable(true),
                );
            }
        });
}
