//! Print snapshots and asynchronous completion (spec 41).
use crate::core::AppCore;
use crate::io::IoResult;
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_platform::printing::{NativePrinter, PrintOutcome, PrintRequest, Printer};
use std::sync::Arc;

pub struct PrintState {
    pub busy: bool,
    pub printer: Arc<dyn Printer>,
}
impl PrintState {
    pub fn new(printer: Option<Arc<dyn Printer>>) -> Self {
        Self {
            busy: false,
            printer: printer.unwrap_or_else(|| Arc::new(NativePrinter)),
        }
    }
}
impl AppCore {
    pub fn print_document(&mut self) {
        if self.printing.busy {
            return;
        }
        let Some(view) = self
            .workbench
            .active_editor()
            .and_then(|id| self.workbench.view(id))
        else {
            return;
        };
        let Some(doc) = self.workbench.document(view.document_id) else {
            return;
        };
        let selected: Vec<_> = view
            .selection
            .ranges()
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| doc.buffer.rope().slice(r.from()..r.to()).to_string())
            .collect();
        let request = PrintRequest {
            title: self.document_title(doc.id),
            text: doc.buffer.text(),
            selection: (!selected.is_empty()).then(|| selected.join("\n")),
            selection_label: crate::t!("printing:selectionOnly"),
        };
        self.printing.busy = true;
        let printer = self.printing.printer.clone();
        let poster = self.io.poster();
        // A print dialog may stay open indefinitely. Never occupy the ordered
        // save/settings worker while waiting for the native process.
        let result = std::thread::Builder::new()
            .name("notoir-print".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    printer.print(request)
                }))
                .unwrap_or_else(|_| Err("Print adapter panicked".into()));
                poster.post(IoResult::Printed(result));
            });
        if let Err(error) = result {
            self.print_finished(Err(error.to_string()));
        }
    }
    pub fn print_finished(&mut self, result: Result<PrintOutcome, String>) {
        self.printing.busy = false;
        match result {
            Ok(PrintOutcome::Cancelled) => {}
            Ok(PrintOutcome::Submitted) => self.notify(Notification::new(
                Severity::Info,
                Translatable::new("printing:submitted"),
            )),
            Err(error) => self.notify(
                Notification::new(Severity::Error, Translatable::new("printing:failed"))
                    .detail(error),
            ),
        }
    }
}
