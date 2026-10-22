//! Open remote text as an editable, local-save-only document.
use crate::{core::AppCore, io::IoResult, t};
use notoir_core::{
    document::{DocumentOrigin, LoadedText},
    workbench::TabPosition,
};
use notoir_editor::{LanguageId, LineEnding, encoding, language, line_ending};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Default)]
pub struct UrlState {
    pub open: bool,
    pub input: String,
    pub error: Option<String>,
    pub request: u64,
    pub cancellation: Option<Arc<AtomicBool>>,
}
impl AppCore {
    pub fn url_cancel(&mut self) {
        if let Some(flag) = self.url.cancellation.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.url.request += 1;
        self.url.open = false;
    }
    pub fn url_start(&mut self, url: String) {
        if self.url.cancellation.is_some() {
            return;
        }
        if !notoir_platform::download::valid_url(url.trim()) {
            self.url.error = Some(t!("url:invalidUrl"));
            return;
        }
        self.url.input = url.trim().to_owned();
        self.url.error = None;
        self.url.request += 1;
        let request = self.url.request;
        let flag = Arc::new(AtomicBool::new(false));
        self.url.cancellation = Some(flag.clone());
        let poster = self.io.poster();
        let url = self.url.input.clone();
        let result = std::thread::Builder::new()
            .name("notoir-download".into())
            .spawn(move || {
                let result = notoir_platform::download::fetch(&url, &flag);
                poster.post(IoResult::UrlDownloaded {
                    request,
                    result: Box::new(result),
                });
            });
        if let Err(e) = result {
            self.url.cancellation = None;
            self.url.error = Some(e.to_string());
        }
    }
    pub fn url_downloaded(
        &mut self,
        request: u64,
        result: Result<notoir_platform::download::Download, String>,
    ) {
        if request != self.url.request || self.url.cancellation.is_none() {
            return;
        }
        self.url.cancellation = None;
        let download = match result {
            Ok(d) => d,
            Err(e) => {
                self.url.error = Some(match e.as_str() {
                    "tooLarge" => t!("url:tooLarge"),
                    "invalidUrl" => t!("url:invalidUrl"),
                    _ => t!("url:failed", error = e),
                });
                return;
            }
        };
        if encoding::looks_binary(&download.bytes)
            || (!download.content_type.is_empty()
                && (download.content_type.starts_with("image/")
                    || download.content_type.starts_with("audio/")
                    || download.content_type.starts_with("video/")
                    || matches!(
                        download.content_type.as_str(),
                        "application/pdf" | "application/zip"
                    )))
        {
            self.url.error = Some(t!("url:binary"));
            return;
        }
        let decoded = encoding::decode(&download.bytes, None);
        if decoded.lossy {
            self.url.error = Some(t!("url:decode"));
            return;
        }
        let eol = line_ending::detect(&decoded.text, LineEnding::platform_default());
        let mut language = language::detect(
            Some(&download.name),
            decoded.text.lines().next().unwrap_or(""),
            &[],
        );
        if language == LanguageId::PLAINTEXT {
            language = match download.content_type.as_str() {
                "text/csv" => LanguageId("csv"),
                "application/json" => LanguageId::JSON,
                "text/markdown" => LanguageId::MARKDOWN,
                _ => language,
            };
        }
        let loaded = LoadedText {
            text: line_ending::normalize(&decoded.text),
            encoding: decoded.encoding,
            line_ending: eol.ending,
            mixed_line_endings: eol.mixed,
            ..LoadedText::empty(language)
        };
        let origin = DocumentOrigin::Untitled {
            n: self.workbench.next_untitled_number(),
        };
        let doc = self.workbench.open_document(origin, loaded);
        if let Some(d) = self.workbench.document_mut(doc) {
            d.suggested_name = Some(download.name);
            d.source_url = Some(download.url);
        }
        let _ = self.workbench.open_view(
            doc,
            self.workbench.layout.active_group,
            TabPosition::AfterActive,
        );
        self.url.open = false;
    }
}
pub fn dialog(ctx: &egui::Context, core: &mut AppCore, actions: &mut crate::shell::Actions) {
    if !core.url.open {
        return;
    }
    egui::Modal::new(egui::Id::new("open-url")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading(t!("url:title"));
        let busy = core.url.cancellation.is_some();
        let response = ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut core.url.input)
                .desired_width(f32::INFINITY)
                .hint_text(t!("url:placeholder")),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, !busy, t!("url:address"))
        });
        crate::live::track(&response, "url.address");
        if let Some(error) = &core.url.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t!("url:loading"));
            });
        }
        let mut send = |id| {
            actions.push(notoir_core::command::CommandInvocation::new(
                id,
                notoir_core::command::InvocationSource::Menu,
            ))
        };
        ui.horizontal(|ui| {
            let open_response = ui.add_enabled(!busy, egui::Button::new(t!("url:open")));
            crate::live::track(&open_response, "url.open");
            if open_response.clicked()
                || (!busy && response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                send("file.downloadUrl");
            }
            let cancel_response = ui.button(t!("url:cancel"));
            crate::live::track(&cancel_response, "url.cancel");
            if cancel_response.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                send("file.cancelUrl");
            }
        });
    });
}
