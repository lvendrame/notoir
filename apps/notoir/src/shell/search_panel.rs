//! Search panel (docs/specs/15 §Workspace Search): query with case / word /
//! regex toggles, replace, include / exclude globs, a summary line and the
//! virtualized result tree.

use egui::text::LayoutJob;
use egui::{FontId, RichText, Sense, TextFormat, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::t;
use crate::theme::Tok;

const ROW_H: f32 = 22.0;

pub fn query_id() -> egui::Id {
    egui::Id::new("search-query")
}

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

fn toggle(ui: &mut egui::Ui, kit: &Kit, on: &mut bool, label: &str, tip: &str) -> bool {
    let text = RichText::new(label)
        .font(crate::fonts::mono(12.0))
        .color(kit.color(if *on {
            Tok::TextPrimary
        } else {
            Tok::TextTertiary
        }));
    let r = ui
        .add(egui::Button::selectable(*on, text).min_size(Vec2::new(24.0, 22.0)))
        .on_hover_text(tip);
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, tip));
    if r.clicked() {
        *on = !*on;
        return true;
    }
    false
}

enum Row {
    File(usize),
    Match(usize, usize),
}

pub fn show(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    let mut changed = false;
    let index_building = core.text_index_building();
    let st = &mut core.search;
    ui.spacing_mut().item_spacing.y = 4.0;
    let row_h = 24.0;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(width, row_h),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            changed |= toggle(ui, kit, &mut st.regex, ".*", &t!("panels:search.regex"));
            changed |= toggle(
                ui,
                kit,
                &mut st.whole_word,
                "ab",
                &t!("panels:search.wholeWord"),
            );
            changed |= toggle(
                ui,
                kit,
                &mut st.case_sensitive,
                "Aa",
                &t!("panels:search.matchCase"),
            );
            let r = ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .id(query_id())
                    .hint_text(t!("panels:search.query"))
                    .desired_width(ui.available_width()),
            );
            if std::mem::take(&mut st.focus_query) {
                r.request_focus();
            }
            if r.changed() {
                changed = true;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                st.due = Some(0);
            }
        },
    );
    ui.allocate_ui_with_layout(
        Vec2::new(width, row_h),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let enabled = st.done && !st.results.is_empty();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(RichText::new(t!("panels:search.replaceAll")).size(12.0)),
                )
                .clicked()
            {
                cmd(actions, "search.replaceAll", serde_json::Value::Null);
            }
            ui.add(
                egui::TextEdit::singleline(&mut st.replace)
                    .hint_text(t!("panels:search.replace"))
                    .desired_width(ui.available_width()),
            );
        },
    );
    let label = |ui: &mut egui::Ui, kit: &Kit, key: &str| {
        ui.label(
            RichText::new(t!(key))
                .size(11.0)
                .color(kit.color(Tok::TextTertiary)),
        );
    };
    label(ui, kit, "panels:search.include");
    if ui
        .add(
            egui::TextEdit::singleline(&mut st.include)
                .hint_text("docs/**, *.md")
                .desired_width(width),
        )
        .changed()
    {
        changed = true;
    }
    label(ui, kit, "panels:search.excludeFiles");
    if ui
        .add(
            egui::TextEdit::singleline(&mut st.exclude)
                .hint_text("**/archive/**")
                .desired_width(width),
        )
        .changed()
    {
        changed = true;
    }
    ui.separator();
    // Summary.
    let total = st.total();
    let summary = if let Some(e) = &st.error {
        RichText::new(e.clone()).color(kit.color(Tok::Danger))
    } else if st.query.is_empty() {
        RichText::new("")
    } else if !st.done || st.due.is_some() {
        RichText::new(t!("panels:search.searching", count = total))
            .color(kit.color(Tok::TextTertiary))
    } else if total == 0 {
        RichText::new(t!("panels:search.noResults")).color(kit.color(Tok::TextTertiary))
    } else {
        RichText::new(t!(
            "panels:search.summary",
            count = total,
            files = st.results.len()
        ))
        .color(kit.color(Tok::TextSecondary))
    };
    ui.horizontal(|ui| {
        ui.label(summary.size(12.0));
        if !st.results.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let all = st.results.iter().all(|f| st.collapsed.contains(&f.rel));
                let (icon, tip) = if all {
                    (Icon::ChevronsUpDown, t!("panels:outline.expandAll"))
                } else {
                    (Icon::ChevronsDownUp, t!("panels:outline.collapseAll"))
                };
                if super::icon_button_sized(ui, kit, icon, &tip, false, true, 22.0).clicked() {
                    if all {
                        st.collapsed.clear();
                    } else {
                        st.collapsed = st.results.iter().map(|f| f.rel.clone()).collect();
                    }
                }
            });
        }
    });
    if st.truncated {
        ui.label(
            RichText::new(t!("panels:search.truncated"))
                .size(11.0)
                .color(kit.color(Tok::Warning)),
        );
    }
    if index_building && !st.query.is_empty() {
        ui.label(
            RichText::new(t!("panels:search.indexing"))
                .size(11.0)
                .color(kit.color(Tok::TextTertiary)),
        );
    }
    let mut rows = Vec::new();
    for (fi, f) in st.results.iter().enumerate() {
        rows.push(Row::File(fi));
        if !st.collapsed.contains(&f.rel) {
            rows.extend((0..f.matches.len()).map(|mi| Row::Match(fi, mi)));
        }
    }
    let mut toggle_file: Option<String> = None;
    let st = &core.search;
    egui::ScrollArea::vertical()
        .id_salt("search-results")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows[range] {
                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                if resp.hovered() {
                    ui.painter()
                        .rect_filled(r, 2.0, kit.color(Tok::BgPanelRaised));
                }
                let cy = r.center().y;
                match *row {
                    Row::File(fi) => {
                        let f = &st.results[fi];
                        let open = !st.collapsed.contains(&f.rel);
                        let icon = if open {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        };
                        let ir = egui::Rect::from_center_size(
                            egui::pos2(r.left() + 8.0, cy),
                            Vec2::splat(12.0),
                        );
                        kit.icon(ui.ctx(), icon, 12.0, kit.color(Tok::TextTertiary))
                            .paint_at(ui, ir);
                        let (dir, name) = match f.rel.rsplit_once('/') {
                            Some((d, n)) => (d, n),
                            None => ("", f.rel.as_str()),
                        };
                        let mut job = LayoutJob::default();
                        job.append(
                            name,
                            0.0,
                            TextFormat::simple(
                                FontId::proportional(13.0),
                                kit.color(Tok::TextPrimary),
                            ),
                        );
                        job.append(
                            dir,
                            8.0,
                            TextFormat::simple(
                                FontId::proportional(11.5),
                                kit.color(Tok::TextTertiary),
                            ),
                        );
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(
                            egui::pos2(r.left() + 18.0, cy - g.size().y / 2.0),
                            g,
                            kit.color(Tok::TextPrimary),
                        );
                        ui.painter().text(
                            egui::pos2(r.right() - 6.0, cy),
                            egui::Align2::RIGHT_CENTER,
                            f.matches.len().to_string(),
                            FontId::proportional(11.5),
                            kit.color(Tok::TextTertiary),
                        );
                        let a11y = t!(
                            "panels:search.fileRow",
                            path = &f.rel,
                            count = f.matches.len()
                        );
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a11y)
                        });
                        if resp.clicked() {
                            toggle_file = Some(f.rel.clone());
                        }
                        let path = f.path.to_string_lossy().into_owned();
                        resp.context_menu(|ui| {
                            if ui.button(t!("panels:search.open")).clicked() {
                                cmd(
                                    actions,
                                    "search.openResult",
                                    json!({ "file": fi, "match": 0 }),
                                );
                                ui.close();
                            }
                            if ui.button(t!("panels:search.openToSide")).clicked() {
                                cmd(
                                    actions,
                                    "search.openResult",
                                    json!({ "file": fi, "match": 0, "side": true }),
                                );
                                ui.close();
                            }
                            if !path.is_empty() && ui.button(t!("panels:search.copyPath")).clicked()
                            {
                                core_copy(actions, &path);
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(t!("panels:search.replaceInFile")).clicked() {
                                cmd(actions, "search.replaceOne", json!({ "file": fi }));
                                ui.close();
                            }
                            if ui.button(t!("panels:search.dismiss")).clicked() {
                                cmd(actions, "search.dismiss", json!({ "file": fi }));
                                ui.close();
                            }
                        });
                    }
                    Row::Match(fi, mi) => {
                        let f = &st.results[fi];
                        let m = &f.matches[mi];
                        let x = r.left() + 24.0;
                        ui.painter().text(
                            egui::pos2(x, cy),
                            egui::Align2::LEFT_CENTER,
                            (m.line + 1).to_string(),
                            crate::fonts::mono(11.5),
                            kit.color(Tok::TextTertiary),
                        );
                        let chars: Vec<char> = m.preview.chars().collect();
                        let (a, b) = (
                            m.preview_range.0.min(chars.len()),
                            m.preview_range.1.min(chars.len()),
                        );
                        let pre: String = chars[..a].iter().collect();
                        let hit: String = chars[a..b].iter().collect();
                        let post: String = chars[b..].iter().collect();
                        let font = FontId::proportional(12.5);
                        let mut job = LayoutJob::default();
                        job.append(
                            pre.trim_start(),
                            0.0,
                            TextFormat::simple(font.clone(), kit.color(Tok::TextSecondary)),
                        );
                        job.append(
                            &hit,
                            0.0,
                            TextFormat {
                                background: kit.color(Tok::EditorFindMatch),
                                ..TextFormat::simple(font.clone(), kit.color(Tok::TextPrimary))
                            },
                        );
                        job.append(
                            &post,
                            0.0,
                            TextFormat::simple(font, kit.color(Tok::TextSecondary)),
                        );
                        let g = ui.painter().layout_job(job);
                        let clip = ui.painter().with_clip_rect(r);
                        clip.galley(
                            egui::pos2(x + 36.0, cy - g.size().y / 2.0),
                            g,
                            kit.color(Tok::TextPrimary),
                        );
                        let a11y = format!("{}:{}", f.rel, m.line + 1);
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a11y)
                        });
                        let resp = resp.on_hover_text(&m.preview);
                        if resp.clicked() {
                            let side = ui.input(|i| i.modifiers.command);
                            cmd(
                                actions,
                                "search.openResult",
                                json!({ "file": fi, "match": mi, "side": side }),
                            );
                        }
                        resp.context_menu(|ui| {
                            if ui.button(t!("panels:search.replaceOne")).clicked() {
                                cmd(
                                    actions,
                                    "search.replaceOne",
                                    json!({ "file": fi, "match": mi }),
                                );
                                ui.close();
                            }
                            if ui.button(t!("panels:search.dismiss")).clicked() {
                                cmd(
                                    actions,
                                    "search.dismiss",
                                    json!({ "file": fi, "match": mi }),
                                );
                                ui.close();
                            }
                        });
                    }
                }
            }
        });
    if let Some(rel) = toggle_file
        && !core.search.collapsed.remove(&rel)
    {
        core.search.collapsed.insert(rel);
    }
    if changed {
        core.search_changed();
    }
}

fn core_copy(actions: &mut Actions, text: &str) {
    cmd(actions, "search.copyPath", json!({ "path": text }));
}
