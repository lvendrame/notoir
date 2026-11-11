mod common;
use egui_kittest::kittest::Queryable;
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_editor::{ChangeSet, EditKind, Range, Selection};
use notoir_platform::printing::{PrintOutcome, PrintRequest, Printer};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct ScriptedPrinter {
    requests: Mutex<Vec<PrintRequest>>,
    answer: Mutex<Option<Result<PrintOutcome, String>>>,
}
impl Printer for ScriptedPrinter {
    fn print(&self, request: PrintRequest) -> Result<PrintOutcome, String> {
        self.requests.lock().unwrap().push(request);
        let start = Instant::now();
        loop {
            if let Some(result) = self.answer.lock().unwrap().take() {
                return result;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "test never answered print dialog"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
fn start() -> (common::TestApp, Arc<ScriptedPrinter>) {
    let printer = Arc::new(ScriptedPrinter::default());
    let adapter = printer.clone();
    (common::start(move |o| o.printer = Some(adapter)), printer)
}
fn edit(app: &mut common::TestApp, text: &str) {
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_view().unwrap();
    core.workbench
        .edit(
            view,
            ChangeSet::insert(0, text),
            Selection::caret(0),
            EditKind::Other,
            1,
        )
        .unwrap();
}
fn wait_until(app: &mut common::TestApp, predicate: impl Fn(&common::TestApp) -> bool) {
    let start = Instant::now();
    while !predicate(app) {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
        app.harness.run_steps(2);
    }
}
fn finish(
    app: &mut common::TestApp,
    printer: &ScriptedPrinter,
    result: Result<PrintOutcome, String>,
) {
    *printer.answer.lock().unwrap() = Some(result);
    wait_until(app, |app| !app.harness.state().core.printing.busy);
}
fn wait_request(app: &mut common::TestApp, printer: &ScriptedPrinter) {
    wait_until(app, |_| !printer.requests.lock().unwrap().is_empty());
}

// spec 41: "Print is visible and disabled without a document"
#[test]
fn menu_is_visible_without_a_document() {
    let (mut app, printer) = start();
    app.harness.get_by_label("File").click();
    app.harness.run_steps(2);
    assert!(app.harness.query_by_label_contains("Print…").is_some());
    let core = &app.harness.state().core;
    let inv = CommandInvocation::new("file.print", InvocationSource::Menu);
    assert!(core.commands.resolve(&inv, &core.context(false)).is_err());
    common::run(&mut app, "file.print", Value::Null);
    assert!(printer.requests.lock().unwrap().is_empty());
}

// spec 41: "Menu and palette use the same print command"
#[test]
fn menu_and_palette_print_active_document() {
    let (mut app, printer) = start();
    common::run(&mut app, "file.newUntitled", Value::Null);
    edit(&mut app, "hello");
    app.harness.get_by_label("File").click();
    app.harness.run_steps(2);
    app.harness.get_by_label_contains("Print…").click();
    app.harness.run_steps(3);
    wait_request(&mut app, &printer);
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
    app.harness.state_mut().core.enqueue(CommandInvocation::new(
        "file.print",
        InvocationSource::Palette,
    ));
    app.harness.run_steps(3);
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
    let requests = printer.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

// spec 41: "Printing captures unsaved text without saving"
// spec 41: "Edits and repeated invocations do not change an open print job"
#[test]
fn unsaved_snapshot_is_immutable_and_duplicate_dialogs_are_prevented() {
    let (mut app, printer) = start();
    common::run(&mut app, "file.newUntitled", Value::Null);
    edit(&mut app, "\t日本語 😀\n\n".repeat(200).as_str());
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    let before = app
        .harness
        .state()
        .core
        .workbench
        .document(doc)
        .unwrap()
        .buffer
        .text();
    common::run(&mut app, "file.print", Value::Null);
    wait_request(&mut app, &printer);
    edit(&mut app, "later");
    common::run(&mut app, "file.print", Value::Null);
    let d = app.harness.state().core.workbench.document(doc).unwrap();
    assert!(d.is_dirty());
    assert!(d.path().is_none());
    assert_eq!(d.buffer.text(), format!("later{before}"));
    let requests = printer.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].text, before);
    assert_eq!(requests[0].selection, None);
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
}

// spec 41: "Selection printing preserves document order"
#[test]
fn snapshot_contains_full_text_and_ordered_selections() {
    let (mut app, printer) = start();
    common::run(&mut app, "file.newUntitled", Value::Null);
    edit(&mut app, "日本語 abc def");
    let view = app.harness.state().core.workbench.active_view().unwrap();
    app.harness
        .state_mut()
        .core
        .workbench
        .set_selection(
            view,
            Selection::new(vec![Range::new(11, 8), Range::new(3, 0)], 0),
        )
        .unwrap();
    common::run(&mut app, "file.print", Value::Null);
    wait_request(&mut app, &printer);
    let request = printer.requests.lock().unwrap()[0].clone();
    assert_eq!(request.text, "日本語 abc def");
    assert_eq!(request.selection.as_deref(), Some("日本語\ndef"));
    assert_eq!(request.selection_label, "Selected text only");
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
}

// spec 41: "Cancellation is silent and failures allow retry"
#[test]
fn cancellation_failure_and_success_have_distinct_results() {
    let (mut app, printer) = start();
    common::run(&mut app, "file.newUntitled", Value::Null);
    let count = app.harness.state().core.notifications.history().count();
    common::run(&mut app, "file.print", Value::Null);
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
    assert_eq!(
        app.harness.state().core.notifications.history().count(),
        count
    );
    common::run(&mut app, "file.print", Value::Null);
    finish(&mut app, &printer, Err("printer unavailable".into()));
    assert!(
        app.harness
            .query_by_label("Could not print the document.")
            .is_some()
    );
    common::run(&mut app, "file.print", Value::Null);
    finish(&mut app, &printer, Ok(PrintOutcome::Submitted));
    assert!(
        app.harness
            .query_by_label("Print job submitted to the system.")
            .is_some()
    );
}

// spec 41: "Read-only and structured documents print their source"
#[test]
fn read_only_preview_prints_source_and_scratchpads_can_print() {
    let (mut app, printer) = start();
    let path = app.home.path().join("source.md");
    std::fs::write(&path, "# Title\n\n**source**\n").unwrap();
    app.harness
        .state_mut()
        .core
        .open_path(&path, Default::default());
    common::settle(&mut app);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .read_only = notoir_core::document::ReadOnly::FileSystem;
    common::run(&mut app, "file.print", Value::Null);
    wait_request(&mut app, &printer);
    assert_eq!(
        printer.requests.lock().unwrap()[0].text,
        "# Title\n\n**source**\n"
    );
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
    let document = app.harness.state().core.workbench.document(doc).unwrap();
    assert!(!document.is_dirty());
    assert_eq!(
        document.read_only,
        notoir_core::document::ReadOnly::FileSystem
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "# Title\n\n**source**\n"
    );
    common::run(&mut app, "file.newScratchpad", Value::Null);
    edit(&mut app, "scratch text");
    printer.requests.lock().unwrap().clear();
    common::run(&mut app, "file.print", Value::Null);
    wait_request(&mut app, &printer);
    assert_eq!(printer.requests.lock().unwrap()[0].text, "scratch text");
    finish(&mut app, &printer, Ok(PrintOutcome::Cancelled));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn native_helper_entry_is_linked_and_rejects_missing_snapshot_before_any_dialog() {
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_notoir"))
        .arg("--notoir-internal-print")
        .arg(directory.path())
        .output()
        .unwrap();
    // Native snapshot/display errors exit 1; the normal CLI parser would
    // reject this private argument with exit 2. No print UI is opened.
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
}
