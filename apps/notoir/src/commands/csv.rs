use super::Specs;
use notoir_core::command::{Category, CommandOutcome, CommandSpec};
pub(super) fn specs(out: &mut Specs) {
    for (id, mode) in [
        ("csv.showEditor", "edit"),
        ("csv.showSplit", "split"),
        ("csv.showPreview", "preview"),
    ] {
        let handler: fn(
            &mut crate::core::AppCore,
            &notoir_core::command::CommandInvocation,
        ) -> CommandOutcome = match mode {
            "edit" => |c, i| set_mode(c, i, "edit"),
            "split" => |c, i| set_mode(c, i, "split"),
            _ => |c, i| set_mode(c, i, "preview"),
        };
        out.push(
            CommandSpec::new(id, Category::View, handler)
                .when("hasActiveEditor && languageId == 'csv'")
                .expect("valid"),
        );
    }
    out.push(
        CommandSpec::new(
            "csv.action",
            Category::Edit,
            |c: &mut crate::core::AppCore, i| {
                c.csv_action(i);
                CommandOutcome::Done
            },
        )
        .hidden(),
    );
}
fn set_mode(
    c: &mut crate::core::AppCore,
    i: &notoir_core::command::CommandInvocation,
    mode: &str,
) -> CommandOutcome {
    let mut inv = i.clone();
    inv.args = serde_json::json!({"op":"mode", "value":mode});
    c.csv_action(&inv);
    CommandOutcome::Done
}
