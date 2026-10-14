//! Floating UI: toasts, notification centre, About and the picker
//! (docs/specs/16 §second-step pickers, 19).

use egui::{Align, Align2, Frame, Key, Layout, Margin, RichText, Stroke};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::notifications::{Notification, Severity};

use super::{Actions, Kit, UiState, icon_button, section_label};
use crate::core::{AppCore, ThemeChoice};
use crate::i18n::{self, Language};
use crate::icons::Icon;
use crate::theme::{ICON, RADIUS_OVERLAY, ThemeId, Tok};
use crate::{fonts, t};

/// A notification's message in the current language.
pub fn message_text(n: &Notification) -> String {
    let args: Vec<(&str, &dyn std::fmt::Display)> = n
        .message
        .args
        .iter()
        .map(|(k, v)| (k.as_str(), v as &dyn std::fmt::Display))
        .collect();
    i18n::tf_in(i18n::language(), &n.message.key, &args)
}

fn overlay_frame(kit: &Kit) -> Frame {
    Frame::new()
        .fill(kit.color(Tok::BgOverlay))
        .stroke(Stroke::new(1.0, kit.color(Tok::BorderStrong)))
        .corner_radius(RADIUS_OVERLAY)
        .inner_margin(Margin::same(10))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: kit.color(Tok::ShadowFloat),
        })
}

fn severity_icon(kit: &Kit, s: Severity) -> (Icon, egui::Color32) {
    match s {
        Severity::Info => (Icon::Info, kit.color(Tok::Info)),
        Severity::Success => (Icon::CircleCheck, kit.color(Tok::Success)),
        Severity::Warning => (Icon::TriangleAlert, kit.color(Tok::Warning)),
        Severity::Error => (Icon::CircleAlert, kit.color(Tok::Danger)),
    }
}

/// Toasts stacked bottom-right above the status bar. Returns hovered ids
/// (their timers pause) and dismissed ids.
pub fn toasts(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &AppCore,
    ui_state: &mut UiState,
    bottom_offset: f32,
    actions: &mut Actions,
) -> (Vec<u64>, Vec<u64>) {
    let mut hovered = Vec::new();
    let mut dismissed = Vec::new();
    let toasts: Vec<Notification> = core.notifications.toasts().into_iter().cloned().collect();
    if toasts.is_empty() {
        return (hovered, dismissed);
    }
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, [-12.0, -(bottom_offset + 12.0)])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(360.0);
            for n in &toasts {
                let resp = overlay_frame(kit).show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.horizontal_top(|ui| {
                        let (icon, color) = severity_icon(kit, n.severity);
                        ui.add(kit.icon(ui.ctx(), icon, ICON, color));
                        let mut text = message_text(n);
                        if n.count > 1 {
                            text.push_str(&format!("  ×{}", n.count));
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(text)
                                    .size(13.0)
                                    .color(kit.color(Tok::TextPrimary)),
                            )
                            .wrap(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if icon_button(ui, kit, Icon::X, &t!("common:close"), false, true)
                                .clicked()
                            {
                                dismissed.push(n.id);
                            }
                        });
                    });
                    if ui_state.expanded_detail == Some(n.id)
                        && let Some(detail) = &n.detail
                    {
                        ui.label(
                            RichText::new(detail)
                                .font(fonts::mono(11.0))
                                .color(kit.color(Tok::TextSecondary)),
                        );
                    }
                    if n.detail.is_some() || !n.actions.is_empty() {
                        ui.horizontal(|ui| {
                            for a in &n.actions {
                                if ui.button(t!(&a.label_key)).clicked() {
                                    actions.push(a.command.clone());
                                    dismissed.push(n.id);
                                }
                            }
                            if n.detail.is_some() && ui.button(t!("common:showDetails")).clicked() {
                                ui_state.expanded_detail = if ui_state.expanded_detail == Some(n.id)
                                {
                                    None
                                } else {
                                    Some(n.id)
                                };
                            }
                        });
                    }
                });
                if resp.response.contains_pointer() {
                    hovered.push(n.id);
                }
                ui.add_space(6.0);
            }
        });
    (hovered, dismissed)
}

/// History popover above the status bar bell.
pub fn notification_centre(
    ctx: &egui::Context,
    kit: &mut Kit,
    core: &AppCore,
    ui_state: &mut UiState,
    bottom_offset: f32,
    actions: &mut Actions,
) {
    if !ui_state.notification_centre_open {
        return;
    }
    let resp = egui::Area::new(egui::Id::new("notification-centre"))
        .anchor(Align2::RIGHT_BOTTOM, [-8.0, -(bottom_offset + 4.0)])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            overlay_frame(kit).show(ui, |ui| {
                ui.set_width(380.0);
                ui.horizontal(|ui| {
                    section_label(ui, kit, &t!("statusbar:notifications"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(t!("statusbar:clearAll")).clicked() {
                            actions.push(CommandInvocation::new(
                                "workbench.clearNotifications",
                                InvocationSource::Internal,
                            ));
                        }
                    });
                });
                ui.separator();
                let items: Vec<Notification> = core.notifications.history().cloned().collect();
                if items.is_empty() {
                    ui.label(
                        RichText::new(t!("statusbar:noNotifications"))
                            .size(13.0)
                            .color(kit.color(Tok::TextTertiary)),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for n in &items {
                            ui.horizontal_top(|ui| {
                                let (icon, color) = severity_icon(kit, n.severity);
                                ui.add(kit.icon(ui.ctx(), icon, ICON, color));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(message_text(n))
                                            .size(13.0)
                                            .color(kit.color(Tok::TextPrimary)),
                                    )
                                    .wrap(),
                                );
                            });
                            ui.add_space(4.0);
                        }
                    });
            });
        });
    // A click on the bell also lands here; the app resolves the bell's toggle
    // against the state at frame start, so it still closes/reopens correctly.
    if resp.response.clicked_elsewhere() {
        ui_state.notification_centre_open = false;
    }
}

pub fn about(ctx: &egui::Context, kit: &mut Kit, ui_state: &mut UiState) {
    if !ui_state.about_open {
        return;
    }
    let resp = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.vertical_centered(|ui| {
            ui.add(kit.icon(ui.ctx(), Icon::AppLogo, 64.0, egui::Color32::WHITE));
            ui.add_space(8.0);
            ui.label(
                RichText::new(crate::BRAND)
                    .font(fonts::extrabold(22.0))
                    .color(kit.color(Tok::TextPrimary)),
            );
            ui.label(
                RichText::new(t!(
                    "dialogs:about.version",
                    version = env!("CARGO_PKG_VERSION")
                ))
                .font(fonts::mono(12.0))
                .color(kit.color(Tok::TextSecondary)),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(t!("dialogs:about.tagline"))
                    .size(13.0)
                    .color(kit.color(Tok::TextSecondary)),
            );
            ui.add_space(12.0);
            ui.label(
                RichText::new(t!("dialogs:about.licenses"))
                    .size(11.0)
                    .color(kit.color(Tok::TextTertiary)),
            );
            ui.add_space(12.0);
            if ui.button(t!("common:close")).clicked() {
                ui.close();
            }
        });
    });
    if resp.should_close() {
        ui_state.about_open = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Theme,
    Language,
    LineEnding,
    /// First step of the encoding picker: Reopen with / Save with.
    EncodingAction,
    Encoding {
        reopen: bool,
    },
    LanguageMode,
}

/// Encodings offered by the encoding picker (WHATWG labels; `utf-8-bom` is ours).
pub const ENCODINGS: &[(&str, &str)] = &[
    ("utf-8", "UTF-8"),
    ("utf-8-bom", "UTF-8 BOM"),
    ("utf-16le", "UTF-16 LE"),
    ("utf-16be", "UTF-16 BE"),
    ("windows-1252", "Western (Windows 1252)"),
    ("iso-8859-15", "Western (ISO 8859-15)"),
    ("windows-1250", "Central European (Windows 1250)"),
    ("windows-1251", "Cyrillic (Windows 1251)"),
    ("koi8-r", "Cyrillic (KOI8-R)"),
    ("windows-1254", "Turkish (Windows 1254)"),
    ("shift_jis", "Japanese (Shift JIS)"),
    ("euc-jp", "Japanese (EUC-JP)"),
    ("gbk", "Simplified Chinese (GBK)"),
    ("big5", "Traditional Chinese (Big5)"),
    ("euc-kr", "Korean (EUC-KR)"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerItem {
    pub value: String,
    pub label: String,
    pub detail: String,
}

pub struct Picker {
    pub kind: PickerKind,
    pub query: String,
    pub selected: usize,
    focus_requested: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// Highlight moved (themes preview live).
    Preview(String),
    Accept(String),
    Cancel,
}

impl Picker {
    pub fn new(kind: PickerKind, core: &AppCore) -> Self {
        let doc = core
            .workbench
            .active_document()
            .and_then(|d| core.workbench.document(d));
        let current = match kind {
            PickerKind::Theme => core.theme_choice.setting_value().to_owned(),
            PickerKind::Language => core.language.code().to_owned(),
            PickerKind::LineEnding => doc
                .map(|d| d.line_ending.label().to_lowercase())
                .unwrap_or_default(),
            PickerKind::EncodingAction => "reopen".to_owned(),
            PickerKind::Encoding { .. } => doc
                .map(|d| {
                    if d.encoding.bom && d.encoding.encoding == encoding_rs::UTF_8 {
                        "utf-8-bom".to_owned()
                    } else {
                        d.encoding.encoding.name().to_lowercase()
                    }
                })
                .unwrap_or_default(),
            PickerKind::LanguageMode => doc
                .map(|d| d.language.as_str().to_owned())
                .unwrap_or_default(),
        };
        let selected = Self::all_items(kind)
            .iter()
            .position(|i| i.value == current)
            .unwrap_or(0);
        Self {
            kind,
            query: String::new(),
            selected,
            focus_requested: false,
        }
    }

    fn all_items(kind: PickerKind) -> Vec<PickerItem> {
        match kind {
            PickerKind::Theme => {
                let mut items: Vec<PickerItem> = ThemeId::ALL
                    .iter()
                    .map(|t| PickerItem {
                        value: t.id().to_owned(),
                        label: t.load().name,
                        detail: String::new(),
                    })
                    .collect();
                items.push(PickerItem {
                    value: ThemeChoice::Auto.setting_value().to_owned(),
                    label: t!("dialogs:picker.themeAuto"),
                    detail: String::new(),
                });
                items
            }
            PickerKind::Language => Language::ALL
                .iter()
                .map(|l| PickerItem {
                    value: l.code().to_owned(),
                    label: l.native_label().to_owned(),
                    detail: l.english_name().to_owned(),
                })
                .collect(),
            PickerKind::LineEnding => [
                ("lf", "LF", "\\n"),
                ("crlf", "CRLF", "\\r\\n"),
                ("cr", "CR", "\\r"),
            ]
            .into_iter()
            .map(|(v, l, d)| PickerItem {
                value: v.to_owned(),
                label: l.to_owned(),
                detail: d.to_owned(),
            })
            .collect(),
            PickerKind::EncodingAction => vec![
                PickerItem {
                    value: "reopen".into(),
                    label: t!("dialogs:picker.reopenWithEncoding"),
                    detail: String::new(),
                },
                PickerItem {
                    value: "save".into(),
                    label: t!("dialogs:picker.saveWithEncoding"),
                    detail: String::new(),
                },
            ],
            PickerKind::Encoding { .. } => ENCODINGS
                .iter()
                .map(|(v, l)| PickerItem {
                    value: (*v).to_owned(),
                    label: (*l).to_owned(),
                    detail: (*v).to_owned(),
                })
                .collect(),
            PickerKind::LanguageMode => notoir_editor::language::LANGUAGES
                .iter()
                .map(|l| PickerItem {
                    value: l.id.as_str().to_owned(),
                    label: if l.id == notoir_editor::LanguageId::PLAINTEXT {
                        t!("statusbar:plainText")
                    } else {
                        l.name.to_owned()
                    },
                    detail: l.id.as_str().to_owned(),
                })
                .collect(),
        }
    }

    /// The command an accepted value runs.
    pub fn accept(kind: PickerKind, value: &str) -> notoir_core::command::CommandInvocation {
        use notoir_core::command::{CommandInvocation, InvocationSource};
        use serde_json::json;
        let (id, args) = match kind {
            PickerKind::Theme => ("preferences.changeTheme", json!({ "theme": value })),
            PickerKind::Language => ("preferences.changeLanguage", json!({ "language": value })),
            PickerKind::LineEnding => ("editor.changeLineEnding", json!({ "eol": value })),
            PickerKind::EncodingAction => ("editor.changeEncoding", json!({ "action": value })),
            PickerKind::Encoding { reopen } => (
                "editor.changeEncoding",
                json!({ "action": if reopen { "reopen" } else { "save" }, "encoding": value }),
            ),
            PickerKind::LanguageMode => ("editor.changeLanguageMode", json!({ "language": value })),
        };
        CommandInvocation::new(id, InvocationSource::Palette).with_args(args)
    }

    /// Items matching the query (case-insensitive, label or detail).
    pub fn items(&self) -> Vec<PickerItem> {
        let q = self.query.to_lowercase();
        Self::all_items(self.kind)
            .into_iter()
            .filter(|i| {
                q.is_empty()
                    || i.label.to_lowercase().contains(&q)
                    || i.detail.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        kit: &mut Kit,
        top_offset: f32,
    ) -> Option<PickerEvent> {
        let mut event = None;
        let items_before = self.items();
        let (down, up, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        let before = self.selected;
        if down && !items_before.is_empty() {
            self.selected = (self.selected + 1) % items_before.len();
        }
        if up && !items_before.is_empty() {
            self.selected = (self.selected + items_before.len() - 1) % items_before.len();
        }
        if esc {
            return Some(PickerEvent::Cancel);
        }
        if enter {
            return items_before
                .get(self.selected)
                .map(|i| PickerEvent::Accept(i.value.clone()));
        }

        let title = match self.kind {
            PickerKind::Theme => t!("dialogs:picker.theme"),
            PickerKind::Language => t!("dialogs:picker.language"),
            PickerKind::LineEnding => t!("dialogs:picker.lineEnding"),
            PickerKind::EncodingAction => t!("dialogs:picker.encodingAction"),
            PickerKind::Encoding { .. } => t!("dialogs:picker.encoding"),
            PickerKind::LanguageMode => t!("dialogs:picker.languageMode"),
        };
        let width = 560.0_f32.min(ctx.content_rect().width() - 32.0);
        let area = egui::Area::new(egui::Id::new("picker"))
            .anchor(Align2::CENTER_TOP, [0.0, top_offset + 8.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                overlay_frame(kit).show(ui, |ui| {
                    ui.set_width(width);
                    section_label(ui, kit, &title);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(t!("dialogs:picker.filter"))
                            .desired_width(f32::INFINITY),
                    );
                    if !self.focus_requested {
                        edit.request_focus();
                        self.focus_requested = true;
                    }
                    if edit.changed() {
                        self.selected = 0;
                    }
                    ui.add_space(4.0);
                    let items = self.items();
                    if items.is_empty() {
                        ui.label(
                            RichText::new(t!("dialogs:picker.noResults"))
                                .size(13.0)
                                .color(kit.color(Tok::TextTertiary)),
                        );
                    }
                    for (i, item) in items.iter().enumerate() {
                        let selected = i == self.selected;
                        let resp = ui.add(
                            egui::Button::selectable(
                                selected,
                                RichText::new(&item.label).size(13.0),
                            )
                            .right_text(
                                RichText::new(&item.detail)
                                    .size(12.0)
                                    .color(kit.color(Tok::TextTertiary)),
                            )
                            .min_size(egui::vec2(width, 26.0)),
                        );
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                            self.selected = i;
                        }
                        if resp.clicked() {
                            event = Some(PickerEvent::Accept(item.value.clone()));
                        }
                    }
                });
            });
        if event.is_none() && area.response.clicked_elsewhere() {
            return Some(PickerEvent::Cancel);
        }
        if event.is_none() && self.selected != before && self.kind == PickerKind::Theme {
            event = self
                .items()
                .get(self.selected)
                .map(|i| PickerEvent::Preview(i.value.clone()));
        }
        event
    }
}
