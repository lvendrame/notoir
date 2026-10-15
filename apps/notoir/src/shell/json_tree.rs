//! JSON Tree view (docs/specs/25 §Tree View): virtualized rows (key or
//! `[index]`, typed value preview, type/count), expand/collapse, filter, node
//! selection mapped to the text, cursor sync, context menu, and the error
//! banner over a greyed-out last valid tree.

use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect, RichText, Sense, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_editor::json::{JsonError, JsonNode, Kind, path_at, paths};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::Effect;
use crate::t;
use crate::theme::Tok;

const ROW_H: f32 = 22.0;
const INDENT: f32 = 14.0;

/// Index path of a node from the root list.
pub type NodePath = Vec<usize>;

#[derive(Default)]
pub struct TreeView {
    pub expanded: HashSet<NodePath>,
    pub selected: Option<NodePath>,
    pub filter: String,
    /// The editor caret last frame (cursor sync).
    pub last_caret: Option<usize>,
    pub scroll_to_selected: bool,
    initialized: bool,
}

#[derive(Default)]
pub struct JsonTrees {
    pub views: HashMap<ViewId, TreeView>,
}

struct Row<'a> {
    path: NodePath,
    node: &'a JsonNode,
    depth: usize,
    /// Array children show `[index]`.
    index_label: Option<usize>,
    expanded: bool,
}

fn get<'a>(roots: &'a [JsonNode], path: &[usize]) -> Option<&'a JsonNode> {
    let mut node = roots.get(*path.first()?)?;
    for &i in &path[1..] {
        node = node.children.get(i)?;
    }
    Some(node)
}

fn matches(n: &JsonNode, q: &str) -> bool {
    n.key
        .as_deref()
        .is_some_and(|k| k.to_lowercase().contains(q))
        || (n.children.is_empty()
            && n.value
                .as_deref()
                .is_some_and(|v| v.to_lowercase().contains(q)))
}

/// Whether `n` or a descendant matches (filter keeps ancestors visible).
fn subtree_matches(n: &JsonNode, q: &str) -> bool {
    matches(n, q) || n.children.iter().any(|c| subtree_matches(c, q))
}

fn flatten<'a>(roots: &'a [JsonNode], st: &TreeView, out: &mut Vec<Row<'a>>) {
    let q = st.filter.trim().to_lowercase();
    fn walk<'a>(
        n: &'a JsonNode,
        path: NodePath,
        depth: usize,
        index: Option<usize>,
        st: &TreeView,
        q: &str,
        out: &mut Vec<Row<'a>>,
    ) {
        if !q.is_empty() && !subtree_matches(n, q) {
            return;
        }
        let container = matches!(n.kind, Kind::Object | Kind::Array);
        let expanded = container
            && (if q.is_empty() {
                st.expanded.contains(&path)
            } else {
                n.children.iter().any(|c| subtree_matches(c, q))
            });
        out.push(Row {
            path: path.clone(),
            node: n,
            depth,
            index_label: index,
            expanded,
        });
        if expanded {
            let array = n.kind == Kind::Array;
            for (i, c) in n.children.iter().enumerate() {
                let mut p = path.clone();
                p.push(i);
                walk(c, p, depth + 1, array.then_some(i), st, q, out);
            }
        }
    }
    for (i, r) in roots.iter().enumerate() {
        walk(r, vec![i], 0, None, st, &q, out);
    }
}

/// Index path of the deepest node containing `offset`.
pub fn path_of_offset(roots: &[JsonNode], offset: usize) -> Option<NodePath> {
    let chain = path_at(roots, offset);
    if chain.is_empty() {
        return None;
    }
    let mut path = Vec::new();
    let mut level = roots;
    for n in &chain {
        let i = level.iter().position(|c| std::ptr::eq(c, *n))?;
        path.push(i);
        level = &n.children;
    }
    Some(path)
}

fn chain_of<'a>(roots: &'a [JsonNode], path: &[usize]) -> Vec<&'a JsonNode> {
    let mut out = Vec::new();
    let Some(mut node) = roots.get(path[0]) else {
        return out;
    };
    out.push(node);
    for &i in &path[1..] {
        match node.children.get(i) {
            Some(c) => {
                node = c;
                out.push(c);
            }
            None => break,
        }
    }
    out
}

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

fn type_label(n: &JsonNode) -> String {
    match n.kind {
        Kind::Object => t!("json:object", count = n.children.len()),
        Kind::Array => t!("json:array", count = n.children.len()),
        Kind::String => t!("json:string"),
        Kind::Number => t!("json:number"),
        Kind::Bool => t!("json:boolean"),
        Kind::Null => "null".to_owned(),
    }
}

/// Renders the tree for `doc` in `ui`.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut crate::core::AppCore,
    view: ViewId,
    doc: DocumentId,
    roots: &[JsonNode],
    error: Option<&JsonError>,
    trees: &mut JsonTrees,
    actions: &mut Actions,
) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 6.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let st = trees.views.entry(view).or_default();
    if !st.initialized && !roots.is_empty() {
        // Roots and their first level start expanded.
        for (i, r) in roots.iter().enumerate() {
            st.expanded.insert(vec![i]);
            if roots.len() == 1 {
                for (j, _) in r.children.iter().enumerate() {
                    st.expanded.insert(vec![i, j]);
                }
            }
        }
        st.initialized = true;
    }
    // Cursor sync: the deepest node at the caret.
    let caret = core
        .workbench
        .views
        .of_document(doc)
        .next()
        .map(|v| v.selection.primary().head);
    // Offsets of a stale tree no longer match the text: no sync while invalid.
    if core.settings.bool("json.syncSelection", None)
        && error.is_none()
        && caret != st.last_caret
        && let Some(c) = caret
    {
        st.last_caret = Some(c);
        if let Some(path) = path_of_offset(roots, c) {
            for k in 1..path.len() {
                st.expanded.insert(path[..k].to_vec());
            }
            st.selected = Some(path);
            st.scroll_to_selected = true;
        }
    }
    // Error banner (click to jump).
    if let Some(e) = error {
        let text = t!(
            "json:invalid",
            line = e.line,
            column = e.column,
            message = &e.message
        );
        let resp = egui::Frame::new()
            .fill(kit.color(Tok::BgPanelRaised))
            .stroke(egui::Stroke::new(1.0, kit.color(Tok::Danger)))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::same(6))
            .show(&mut ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(
                        RichText::new(&text)
                            .size(12.0)
                            .color(kit.color(Tok::Danger)),
                    )
                    .sense(Sense::click()),
                )
            })
            .inner;
        if resp.clicked() {
            cmd(
                actions,
                "json.selectRange",
                json!({ "doc": doc.0, "start": e.offset, "end": e.offset }),
            );
        }
        ui.add_space(4.0);
    }
    // Filter.
    ui.add(
        egui::TextEdit::singleline(&mut st.filter)
            .hint_text(t!("json:filter"))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    let large = core.workbench.document(doc).is_some_and(|d| {
        d.buffer.len_chars() as i64 > core.settings.int("json.tree.maxSize", None)
    });
    if large {
        ui.label(RichText::new(t!("json:tooLarge")).color(kit.color(Tok::TextTertiary)));
        return;
    }
    let mut rows = Vec::new();
    flatten(roots, st, &mut rows);
    let dim = error.is_some();
    let selected = st.selected.clone();
    let scroll_to = std::mem::take(&mut st.scroll_to_selected)
        .then(|| rows.iter().position(|r| Some(&r.path) == selected.as_ref()))
        .flatten();
    let mut toggle: Option<NodePath> = None;
    let mut select: Option<(NodePath, bool)> = None;
    let mut expand_below: Option<(NodePath, bool)> = None;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("json-tree", view))
        .auto_shrink([false, false]);
    if let Some(i) = scroll_to {
        let h = ui.available_height();
        scroll = scroll.vertical_scroll_offset((i as f32 * ROW_H - h / 2.0).max(0.0));
    }
    let text_alpha = if dim { 0.45 } else { 1.0 };
    let type_color = |kit: &Kit, k: Kind| match k {
        Kind::String => kit.color(Tok::SyntaxString),
        Kind::Number => kit.color(Tok::SyntaxNumber),
        Kind::Bool | Kind::Null => kit.color(Tok::SyntaxConstant),
        _ => kit.color(Tok::TextTertiary),
    };
    scroll.show_rows(&mut ui, ROW_H, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &rows[range] {
            let (r, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            let is_sel = Some(&row.path) == selected.as_ref();
            if is_sel {
                ui.painter().rect_filled(r, 2.0, kit.color(Tok::AccentSoft));
            } else if resp.hovered() {
                ui.painter()
                    .rect_filled(r, 2.0, kit.color(Tok::BgPanelRaised));
            }
            let mut x = r.left() + 4.0 + row.depth as f32 * INDENT;
            let cy = r.center().y;
            let container = matches!(row.node.kind, Kind::Object | Kind::Array);
            if container {
                let icon = if row.expanded {
                    crate::icons::Icon::ChevronDown
                } else {
                    crate::icons::Icon::ChevronRight
                };
                let ir = Rect::from_center_size(Pos2::new(x + 6.0, cy), Vec2::splat(12.0));
                kit.icon(ui.ctx(), icon, 12.0, kit.color(Tok::TextTertiary))
                    .paint_at(ui, ir);
                if resp.clicked()
                    && ui
                        .input(|i| i.pointer.interact_pos())
                        .is_some_and(|p| p.x < x + 14.0)
                {
                    toggle = Some(row.path.clone());
                }
            }
            x += 16.0;
            let painter = ui.painter_at(r);
            let key = match (&row.node.key, row.index_label, row.depth) {
                (Some(k), _, _) => k.clone(),
                (None, Some(i), _) => format!("[{i}]"),
                (None, None, _) => t!("json:root"),
            };
            let key_color = kit.color(Tok::SyntaxProperty).gamma_multiply(text_alpha);
            let kg = painter.layout_no_wrap(key.clone(), egui::FontId::monospace(12.5), key_color);
            let kw = kg.size().x;
            painter.galley(Pos2::new(x, cy - kg.size().y / 2.0), kg, key_color);
            let preview = match row.node.kind {
                Kind::Object => "{}".to_owned(),
                Kind::Array => "[]".to_owned(),
                Kind::String => format!("\"{}\"", row.node.value.as_deref().unwrap_or("")),
                _ => row.node.value.clone().unwrap_or_default(),
            };
            let short: String = if preview.chars().count() > 80 {
                preview.chars().take(77).chain("…".chars()).collect()
            } else {
                preview.clone()
            };
            let vx = (x + kw + 16.0).max(x + 140.0);
            let vcolor = type_color(kit, row.node.kind).gamma_multiply(text_alpha);
            painter.text(
                Pos2::new(vx, cy),
                egui::Align2::LEFT_CENTER,
                &short,
                egui::FontId::monospace(12.5),
                vcolor,
            );
            painter.text(
                Pos2::new(r.right() - 6.0, cy),
                egui::Align2::RIGHT_CENTER,
                type_label(row.node),
                egui::FontId::proportional(11.5),
                kit.color(Tok::TextTertiary).gamma_multiply(text_alpha),
            );
            let resp = resp.on_hover_text(&preview);
            let a11y = format!("{key}: {short}");
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, is_sel, &a11y)
            });
            if resp.clicked() && toggle.is_none() {
                select = Some((row.path.clone(), ui.input(|i| i.modifiers.shift)));
            }
            if resp.double_clicked() && container {
                toggle = Some(row.path.clone());
            }
            resp.context_menu(|ui| {
                let chain = chain_of(roots, &row.path);
                let (jp, js) = paths(&chain);
                let mut copy = |ui: &mut egui::Ui, label: &str, text: String| {
                    if ui.button(t!(label)).clicked() {
                        core.effects.push(Effect::CopyText(text));
                        ui.close();
                    }
                };
                let value_text: String = core
                    .workbench
                    .document(doc)
                    .map(|d| {
                        d.buffer
                            .rope()
                            .slice(row.node.range.0..row.node.range.1.min(d.buffer.len_chars()))
                            .to_string()
                    })
                    .unwrap_or_default();
                copy(ui, "json:copyValue", value_text);
                if let Some(k) = &row.node.key {
                    copy(ui, "json:copyKey", k.clone());
                }
                copy(ui, "json:copyPath", jp);
                copy(ui, "json:copyJsPath", js);
                ui.separator();
                if container {
                    if ui.button(t!("json:expandAllBelow")).clicked() {
                        expand_below = Some((row.path.clone(), true));
                        ui.close();
                    }
                    if ui.button(t!("json:collapseAllBelow")).clicked() {
                        expand_below = Some((row.path.clone(), false));
                        ui.close();
                    }
                }
                if ui.button(t!("json:revealInText")).clicked() {
                    select = Some((row.path.clone(), false));
                    ui.close();
                }
            });
        }
    });
    // `*` expands everything below the selected node.
    if ui.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Text(t) if t == "*"))
    }) && ui.ui_contains_pointer()
        && let Some(p) = selected.clone()
    {
        expand_below = Some((p, true));
    }
    let st = trees.views.entry(view).or_default();
    if let Some(p) = toggle
        && !st.expanded.remove(&p)
    {
        st.expanded.insert(p);
    }
    if let Some((p, expand)) = expand_below
        && let Some(n) = get(roots, &p)
    {
        fn all(n: &JsonNode, path: NodePath, out: &mut Vec<NodePath>) {
            if matches!(n.kind, Kind::Object | Kind::Array) {
                out.push(path.clone());
                for (i, c) in n.children.iter().enumerate() {
                    let mut q = path.clone();
                    q.push(i);
                    all(c, q, out);
                }
            }
        }
        let mut paths = Vec::new();
        all(n, p, &mut paths);
        for q in paths {
            if expand {
                st.expanded.insert(q);
            } else {
                st.expanded.remove(&q);
            }
        }
    }
    if let Some((p, with_key)) = select
        && let Some(n) = get(roots, &p)
    {
        st.selected = Some(p);
        let start = if with_key {
            n.key_range.map_or(n.range.0, |k| k.0)
        } else {
            n.range.0
        };
        cmd(
            actions,
            "json.selectRange",
            json!({ "doc": doc.0, "start": start, "end": n.range.1 }),
        );
        // Selecting in the tree must not bounce back through cursor sync.
        st.last_caret = Some(n.range.1);
    }
}
