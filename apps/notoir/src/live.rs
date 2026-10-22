//! Opt-in desktop test bridge. Drives egui input in the running native app.
//! Disabled unless NOTOIR_TEST_COMMANDS=1; never used by product commands.
use serde_json::{Value, json};
use std::cell::RefCell;
thread_local! { static WIDGETS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) }; }
pub fn enabled() -> bool {
    std::env::var("NOTOIR_TEST_COMMANDS").as_deref() == Ok("1")
}
pub fn begin() {
    if enabled() {
        WIDGETS.with(|w| w.borrow_mut().clear());
    }
}
pub fn track(response: &egui::Response, label: &str) {
    if enabled() && response.enabled() {
        WIDGETS.with(|w| {
            w.borrow_mut().push(
                json!({"label":label,"x":response.rect.center().x,"y":response.rect.center().y}),
            )
        });
    }
}
pub fn input(core: &mut crate::core::AppCore, _ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !enabled() {
        return;
    }
    for request in std::mem::take(&mut core.live_requests) {
        let op = request["op"].as_str().unwrap_or("");
        if op == "savePath" {
            if let Some(path) = request["path"].as_str() {
                let dialogs = std::sync::Arc::new(crate::files::ScriptedDialogs::default());
                dialogs.answer_save(Some(path.into()));
                core.files.dialogs = dialogs;
            }
            continue;
        }
        if op == "capture" {
            core.live_capture = request["path"].as_str().map(str::to_owned);
            continue;
        }
        if let Some(path) = request["output"].as_str() {
            core.live_output = Some(path.to_owned());
        }
        if op == "doubleClick" {
            let mut second = request.clone();
            second["op"] = json!("click");
            core.live_requests.push(second);
        }
        if op == "click" || op == "down" || op == "up" || op == "doubleClick" {
            let target = request["target"].as_str();
            let position = WIDGETS.with(|w| {
                w.borrow()
                    .iter()
                    .rev()
                    .find(|v| v["label"].as_str() == target)
                    .cloned()
            });
            let x = position
                .as_ref()
                .and_then(|v| v["x"].as_f64())
                .or_else(|| request["x"].as_f64());
            let y = position
                .as_ref()
                .and_then(|v| v["y"].as_f64())
                .or_else(|| request["y"].as_f64());
            if let (Some(x), Some(y)) = (x, y) {
                let pos = egui::pos2(x as f32, y as f32);
                raw.events.push(egui::Event::PointerMoved(pos));
                for pressed in [true, false] {
                    if (op == "down" && !pressed) || (op == "up" && pressed) {
                        continue;
                    }
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        } else if op == "text" {
            raw.events.push(egui::Event::Text(
                request["text"].as_str().unwrap_or("").to_owned(),
            ));
        } else if op == "key" {
            let key = match request["key"].as_str() {
                Some("Enter") => egui::Key::Enter,
                Some("Escape") => egui::Key::Escape,
                Some("Tab") => egui::Key::Tab,
                Some("Backspace") => egui::Key::Backspace,
                Some("Z") => egui::Key::Z,
                Some("S") => egui::Key::S,
                _ => egui::Key::A,
            };
            let command = request["command"].as_bool().unwrap_or(false);
            let modifiers = egui::Modifiers {
                command,
                mac_cmd: command && cfg!(target_os = "macos"),
                ctrl: command && !cfg!(target_os = "macos"),
                shift: request["shift"].as_bool().unwrap_or(false),
                ..Default::default()
            };
            raw.events.push(egui::Event::ModifiersChanged(modifiers));
            for pressed in [true, false] {
                raw.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                });
            }
        }
    }
}
pub fn finish(core: &mut crate::core::AppCore, ctx: &egui::Context) {
    if !enabled() {
        return;
    }
    if !core.live_requests.is_empty() {
        ctx.request_repaint();
    }
    if let Some(path) = core.live_capture.take() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(path)));
        ctx.request_repaint();
    }
    if let Some(path) = core.live_output.take() {
        let docs:Vec<_>=core.workbench.documents.values().filter(|d| !matches!(d.origin,notoir_core::document::DocumentOrigin::Virtual{..})).map(|d|json!({"id":d.id.0,"name":core.document_title(d.id),"text":d.buffer.text(),"dirty":d.is_dirty(),"source_url":d.source_url,"path":d.path()})).collect();
        let view = core.workbench.active_view();
        let csv=view.and_then(|v|core.csv.views.get(&v)).map(|s| {
            let rows=s.parsed.as_ref().map(|t|s.visible_rows(t)).unwrap_or_default();
            let matches=s.parsed.as_ref().map(|t|s.matches(t,&rows)).unwrap_or_default();
            json!({"header":s.header,"separator":s.detected.to_string(),"hidden":s.hidden,"filters":s.filters,"rows":rows,"matches":matches,"editing":s.editing.is_some(),"error":s.error.is_some(),"pending":s.pending.is_some()})
        });
        let widgets = WIDGETS.with(|w| w.borrow().clone());
        let value = json!({"documents":docs,"view":view.map(|v|v.0),"mode":view.and_then(|v|core.workbench.view(v)).map(|v|format!("{:?}",v.kind)),"csv":csv,"url_open":core.url.open,"url_busy":core.url.cancellation.is_some(),"url_error":core.url.error,"widgets":widgets});
        core.io.submit(move || {
            let _ = std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap_or_default());
            None
        });
    }
    let screenshots: Vec<_> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| {
                if let egui::Event::Screenshot {
                    user_data, image, ..
                } = e
                {
                    Some((user_data.clone(), image.clone()))
                } else {
                    None
                }
            })
            .collect()
    });
    for (data, image) in screenshots {
        if let Some(path) = data.data.and_then(|d| d.downcast_ref::<String>().cloned()) {
            core.io.submit(move || {
                let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let _ = image::save_buffer(
                    path,
                    &rgba,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                );
                None
            });
        }
    }
}
