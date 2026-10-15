//! JSON commands (docs/specs/25).

use notoir_core::command::{Category, CommandInvocation, CommandOutcome, CommandSpec};
use notoir_core::ids::DocumentId;

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
            .when(json)
            .expect("valid"),
    );
    out.push(CommandSpec::new("json.selectRange", Category::Json, select_range).hidden());
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
            core.select_json_range(d, (a as usize, b as usize));
            CommandOutcome::Done
        }
        _ => bad_args(inv),
    }
}
