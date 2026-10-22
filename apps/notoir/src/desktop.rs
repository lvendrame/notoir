//! File-manager registration runs on the IO worker, never in the menu renderer.
use crate::{core::AppCore, io::IoResult, t};
use notoir_platform::file_manager::Integration;
#[derive(Default)]
pub struct DesktopState {
    pub integration: Option<Integration>,
    pub installed: Option<bool>,
    pub busy: bool,
    pub error: Option<String>,
    pub open: bool,
}
impl AppCore {
    pub fn desktop_change(&mut self, install: Option<bool>) {
        if self.desktop.busy {
            return;
        }
        self.desktop.busy = true;
        self.desktop.error = None;
        let label = t!("integration:label");
        let integration = self.desktop.integration.clone();
        self.io.submit(move || {
            let result = (|| {
                let integration = match integration {
                    Some(i) => i,
                    None => Integration::native(label)?,
                };
                if let Some(add) = install {
                    integration.change(add)?;
                }
                integration.installed()
            })();
            Some(IoResult::DesktopIntegration(result))
        });
    }
    pub fn desktop_done(&mut self, result: Result<bool, String>) {
        self.desktop.busy = false;
        match result {
            Ok(installed) => self.desktop.installed = Some(installed),
            Err(e) => self.desktop.error = Some(e),
        }
    }
}
pub fn dialog(ctx: &egui::Context, core: &mut AppCore, actions: &mut crate::shell::Actions) {
    if !core.desktop.open {
        return;
    }
    egui::Modal::new(egui::Id::new("file-manager-integration")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading(t!("integration:title"));
        ui.label(if cfg!(target_os = "macos") {
            t!("integration:mac")
        } else if cfg!(windows) {
            t!("integration:windows")
        } else {
            t!("integration:linux")
        });
        if core.desktop.busy {
            ui.spinner();
        }
        if let Some(e) = &core.desktop.error {
            ui.colored_label(ui.visuals().error_fg_color, t!("integration:failed"));
            ui.label(e);
        }
        if let Some(installed) = core.desktop.installed {
            ui.label(if installed {
                t!("integration:installed")
            } else {
                t!("integration:removed")
            });
        }
        ui.horizontal(|ui| {
            let mut send = |id| {
                actions.push(notoir_core::command::CommandInvocation::new(
                    id,
                    notoir_core::command::InvocationSource::Menu,
                ))
            };
            for (id, key, enabled) in [
                (
                    "app.addContextMenu",
                    "integration:add",
                    core.desktop.installed != Some(true),
                ),
                (
                    "app.removeContextMenu",
                    "integration:remove",
                    core.desktop.installed == Some(true),
                ),
            ] {
                let r = ui.add_enabled(!core.desktop.busy && enabled, egui::Button::new(t!(key)));
                crate::live::track(&r, id);
                if r.clicked() {
                    send(id);
                }
            }
            let r = ui.button(t!("integration:close"));
            crate::live::track(&r, "integration.close");
            if r.clicked() {
                send("app.closeContextMenu");
            }
        });
    });
}
