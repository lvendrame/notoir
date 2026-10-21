//! JSON commands (docs/specs/25).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::{DocumentId, ViewId};

use super::{Specs, bad_args};
use crate::core::AppCore;

pub(super) fn specs(out: &mut Specs) {
    let json = "hasActiveEditor && (languageId == 'json' || languageId == 'jsonc' || languageId == 'jsonl')";
    let spec = |id: &'static str, f: fn(&mut AppCore, &CommandInvocation) -> CommandOutcome| {
        CommandSpec::new(id, Category::Json, f)
            .when(json)
            .expect("valid")
    };
    out.push(spec("json.showText", |c, _| mode(c, "text")));
    out.push(spec("json.showSplit", |c, _| mode(c, "split")));
    out.push(spec("json.showTree", |c, _| mode(c, "tree")));
    out.push(spec("json.format", |c, _| transform(c, "format")).icon("braces"));
    out.push(spec("json.minify", |c, _| transform(c, "minify")));
    out.push(spec("json.sortKeys", |c, _| transform(c, "sort")));
    out.push(spec("json.goToParent", go_to_parent));
    out.push(
        CommandSpec::new("editor.formatDocument", Category::Edit, format_document)
            .when(&format!(
                "({json}) || (hasActiveEditor && languageId == 'yaml') || pluginFormatter"
            ))
            .expect("valid"),
    );
    let yaml = "hasActiveEditor && languageId == 'yaml'";
    for (id, handler) in [
        (
            "yaml.showText",
            (|c: &mut AppCore, _: &CommandInvocation| mode(c, "text"))
                as fn(&mut AppCore, &CommandInvocation) -> CommandOutcome,
        ),
        ("yaml.showSplit", |c, _| mode(c, "split")),
        ("yaml.showTree", |c, _| mode(c, "tree")),
        ("yaml.format", |c, _| transform(c, "format")),
        ("yaml.goToParent", go_to_parent),
    ] {
        out.push(
            CommandSpec::new(id, Category::Yaml, handler)
                .when(yaml)
                .expect("valid"),
        );
    }
    out.push(CommandSpec::new("yaml.selectRange", Category::Yaml, select_range).hidden());
    out.push(CommandSpec::new("yaml.treeEdit", Category::Yaml, tree_edit).hidden());
    out.push(CommandSpec::new("json.selectRange", Category::Json, select_range).hidden());
    out.push(CommandSpec::new("json.treeEdit", Category::Json, tree_edit).hidden());
}

/// `{ view, path: [..], op: "value" | "key" | "add" | "delete" | "move",
/// text?, kind?, delta? }`: a JSON tree edit (spec 25).
fn tree_edit(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    use crate::json::TreeOp;
    let a = &inv.args;
    let view = a.get("view").and_then(|v| v.as_u64()).map(ViewId);
    let path: Option<Vec<usize>> = a
        .get("path")
        .and_then(|p| p.as_array())
        .and_then(|p| p.iter().map(|i| i.as_u64().map(|i| i as usize)).collect());
    let text = || a.get("text").and_then(|t| t.as_str()).map(str::to_owned);
    let op = match a.get("op").and_then(|o| o.as_str()) {
        Some("value") => text().map(TreeOp::Value),
        Some("key") => text().map(TreeOp::Key),
        Some("add") => a
            .get("kind")
            .and_then(|k| k.as_str())
            .and_then(notoir_editor::json_edit::NewValue::from_key)
            .map(TreeOp::Add),
        Some("delete") => Some(TreeOp::Delete),
        Some("move") => a
            .get("delta")
            .and_then(|d| d.as_i64())
            .filter(|d| d.abs() == 1)
            .map(|d| TreeOp::Move(d as isize)),
        _ => None,
    };
    match (view, path, op) {
        (Some(view), Some(path), Some(op)) if !path.is_empty() => {
            core.json_tree_edit(view, &path, op);
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}

fn mode(core: &mut AppCore, m: &str) -> CommandOutcome {
    if let Some(v) = core.workbench.active_view() {
        core.set_json_mode(v, m);
    }
    CommandOutcome::Done
}

fn transform(core: &mut AppCore, op: &str) -> CommandOutcome {
    if let Some(v) = core.workbench.active_view() {
        core.json_transform(v, op);
    }
    CommandOutcome::Done
}

/// Format Document (editor context menu): the JSON formatter for now;
/// plugin formatters join later (spec 30).
fn format_document(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    // A plugin formatter for the language wins (spec 30).
    if let Some(v) = core.workbench.active_editor()
        && core.plugin_format(v)
    {
        return CommandOutcome::Done;
    }
    transform(core, "format")
}

fn go_to_parent(core: &mut AppCore, _: &CommandInvocation) -> CommandOutcome {
    if let Some(v) = core.workbench.active_view() {
        core.json_go_to_parent(v);
    }
    CommandOutcome::Done
}

/// `{ doc, start, end }`.
fn select_range(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let doc = inv.args.get("doc").and_then(|v| v.as_u64()).map(DocumentId);
    let start = inv.args.get("start").and_then(|v| v.as_u64());
    let end = inv.args.get("end").and_then(|v| v.as_u64());
    match (doc, start, end) {
        (Some(d), Some(a), Some(b)) => {
            let view = inv.args.get("view").and_then(|v| v.as_u64()).map(ViewId);
            core.select_json_range(d, view, (a as usize, b as usize));
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}
