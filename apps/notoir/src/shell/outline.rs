//! Outline panel (docs/specs/26 §Outline): the active document's symbols as a
//! tree with filter, sort by position or name, collapse/expand all, the
//! symbol at the cursor highlighted, click to jump.

use std::collections::HashSet;

use egui::{RichText, Sense, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::DocumentId;
use notoir_editor::outline::{Symbol, SymbolKind, path_at};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::icons::Icon;
use crate::t;
use crate::theme::Tok;

const ROW_H: f32 = 22.0;

#[derive(Default)]
pub struct OutlineUi {
    pub filter: String,
    pub sort_by_name: bool,
    /// Collapsed symbols by (document, index path).
    pub collapsed: HashSet<(DocumentId, Vec<usize>)>,
}

struct Row<'a> {
    sym: &'a Symbol,
    path: Vec<usize>,
    depth: usize,
    has_children: bool,
}

fn visible_children(s: &Symbol, depth: usize, max_key_depth: usize) -> bool {
    !s.children.is_empty()
        && (matches!(s.kind, SymbolKind::Heading(_)) || depth + 1 < max_key_depth)
}

fn matches(s: &Symbol, q: &str) -> bool {
    s.name.to_lowercase().contains(q) || s.children.iter().any(|c| matches(c, q))
}

#[allow(clippy::too_many_arguments)]
fn rows<'a>(
    syms: &'a [Symbol],
    st: &OutlineUi,
    doc: DocumentId,
    max_key_depth: usize,
    q: &str,
    depth: usize,
    prefix: &[usize],
    out: &mut Vec<Row<'a>>,
) {
    let mut order: Vec<usize> = (0..syms.len()).collect();
    if st.sort_by_name {
        order.sort_by_key(|&i| syms[i].name.to_lowercase());
    }
    for i in order {
        let s = &syms[i];
        if !q.is_empty() && !matches(s, q) {
            continue;
        }
        let mut path = prefix.to_vec();
        path.push(i);
        let has_children = visible_children(s, depth, max_key_depth);
        let open = !st.collapsed.contains(&(doc, path.clone())) || !q.is_empty();
        out.push(Row {
            sym: s,
            path: path.clone(),
            depth,
            has_children,
        });
        if has_children && open {
            rows(
                &s.children,
                st,
                doc,
                max_key_depth,
                q,
                depth + 1,
                &path,
                out,
            );
        }
    }
}

fn all_paths(syms: &[Symbol], prefix: &[usize], out: &mut Vec<Vec<usize>>) {
    for (i, s) in syms.iter().enumerate() {
        let mut p = prefix.to_vec();
        p.push(i);
        if !s.children.is_empty() {
            out.push(p.clone());
            all_paths(&s.children, &p, out);
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    st: &mut OutlineUi,
    actions: &mut Actions,
) {
    let Some(view) = core.workbench.active_view() else {
        hint(ui, kit, &t!("panels:outline.noEditor"));
        return;
    };
    let Some(doc) = core.workbench.view(view).map(|v| v.document_id) else {
        return;
    };
    let Some(s) = core.structure(doc) else { return };
    let caret = core
        .workbench
        .view(view)
        .map_or(0, |v| v.selection.primary().head);
    // Toolbar: filter, sort, collapse/expand all.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let w = (ui.available_width() - 56.0).max(60.0);
        ui.add(
            egui::TextEdit::singleline(&mut st.filter)
                .hint_text(t!("panels:outline.filter"))
                .desired_width(w),
        );
        let sort_tip = if st.sort_by_name {
            t!("panels:outline.sortByPosition")
        } else {
            t!("panels:outline.sortByName")
        };
        if super::icon_button_sized(
            ui,
            kit,
            Icon::ArrowDownAZ,
            &sort_tip,
            st.sort_by_name,
            true,
            24.0,
        )
        .clicked()
        {
            st.sort_by_name = !st.sort_by_name;
        }
        let any_open = {
            let mut paths = Vec::new();
            all_paths(&s.symbols, &[], &mut paths);
            paths
                .iter()
                .any(|p| !st.collapsed.contains(&(doc, p.clone())))
        };
        let (icon, tip) = if any_open {
            (Icon::ChevronsDownUp, t!("panels:outline.collapseAll"))
        } else {
            (Icon::ChevronsUpDown, t!("panels:outline.expandAll"))
        };
        if super::icon_button_sized(ui, kit, icon, &tip, false, true, 24.0).clicked() {
            let mut paths = Vec::new();
            all_paths(&s.symbols, &[], &mut paths);
            for p in paths {
                if any_open {
                    st.collapsed.insert((doc, p));
                } else {
                    st.collapsed.remove(&(doc, p));
                }
            }
        }
    });
    ui.add_space(4.0);
    if s.symbols.is_empty() {
        hint(ui, kit, &t!("panels:outline.noSymbols"));
        return;
    }
    let max_key_depth = core.settings.int("outline.depth", None).max(1) as usize;
    let q = st.filter.trim().to_lowercase();
    let mut list = Vec::new();
    rows(&s.symbols, st, doc, max_key_depth, &q, 0, &[], &mut list);
    // The symbol containing the cursor.
    let current: Option<*const Symbol> = core
        .settings
        .bool("outline.followCursor", None)
        .then(|| {
            path_at(&s.symbols, caret)
                .last()
                .map(|x| *x as *const Symbol)
        })
        .flatten();
    let mut toggle = None;
    egui::ScrollArea::vertical()
        .id_salt("outline")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, list.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &list[range] {
                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                let is_current = current == Some(row.sym as *const Symbol);
                if is_current {
                    ui.painter().rect_filled(r, 2.0, kit.color(Tok::AccentSoft));
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(r, 2.0, kit.color(Tok::BgPanelRaised));
                }
                let x = r.left() + 4.0 + row.depth as f32 * 14.0;
                if row.has_children {
                    let open = !st.collapsed.contains(&(doc, row.path.clone())) || !q.is_empty();
                    let icon = if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    };
                    let ir = egui::Rect::from_center_size(
                        egui::pos2(x + 6.0, r.center().y),
                        Vec2::splat(12.0),
                    );
                    kit.icon(ui.ctx(), icon, 12.0, kit.color(Tok::TextTertiary))
                        .paint_at(ui, ir);
                    if resp.clicked()
                        && ui
                            .input(|i| i.pointer.interact_pos())
                            .is_some_and(|p| p.x < x + 14.0)
                    {
                        toggle = Some(row.path.clone());
                        continue;
                    }
                }
                let color = kit.color(if is_current {
                    Tok::TextPrimary
                } else {
                    Tok::TextSecondary
                });
                ui.painter().text(
                    egui::pos2(x + 16.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    &row.sym.name,
                    egui::FontId::proportional(13.0),
                    color,
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        is_current,
                        &row.sym.name,
                    )
                });
                if resp.clicked() {
                    let at = row.sym.selection_range.0;
                    actions.push(
                        CommandInvocation::new("navigate.revealRange", InvocationSource::Internal)
                            .with_args(json!({ "start": at, "end": at, "view": view.0 })),
                    );
                }
            }
        });
    if let Some(p) = toggle
        && !st.collapsed.remove(&(doc, p.clone()))
    {
        st.collapsed.insert((doc, p));
    }
}

fn hint(ui: &mut egui::Ui, kit: &mut Kit, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(kit.color(Tok::TextTertiary)),
    );
}
