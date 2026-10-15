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
    /// The Command Palette (spec 16).
    Commands,
    /// Status bar indentation item: spaces/tabs, tab size, detect, convert.
    Indentation,
    /// Second step of Indentation: 1–8.
    TabSize,
    /// Go to Symbol in the active document (`@`).
    Symbols,
    /// Quick Open (`Mod+P`): files, with `>` `:` `@` `?` prefixes.
    QuickOpen,
    /// Compare: a side from open documents, workspace files or the
    /// clipboard (spec 27).
    Compare,
    /// File History: commits touching a file (spec 28).
    GitHistory,
    /// Switch Branch / Create Branch From… (spec 28).
    GitBranches,
    /// Create Branch: the new branch's name.
    GitBranchName,
    /// Clone Repository: URL (recent ones listed).
    GitCloneUrl,
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
    /// Command Palette rows (empty for other pickers).
    commands: Vec<CommandEntry>,
    /// Also list commands that are disabled right now (dimmed).
    show_all: bool,
    /// Context when the palette opened; commands run with it.
    pub context: Option<notoir_core::when::ContextSnapshot>,
    scroll_to_selected: bool,
    /// Rows computed when the picker opened (symbols).
    dynamic: Vec<PickerItem>,
    /// Quick Open: what it lists, and the rows for the current query.
    quick: Option<crate::quick_open::Source>,
    quick_rows: Option<(String, Vec<crate::quick_open::Row>)>,
    /// Commands / Symbols reached from Quick Open by typing a prefix:
    /// deleting it goes back.
    pub from_quick_open: bool,
    /// Compare: which side is being chosen.
    compare_hint: Option<String>,
    /// Go to Symbol: where to go back to on Esc.
    pub restore: Option<(
        notoir_core::ids::ViewId,
        notoir_editor::Selection,
        notoir_core::view::ScrollPosition,
    )>,
}

/// One Command Palette row.
#[derive(Clone, Debug)]
pub struct CommandEntry {
    pub id: String,
    /// "Category: Title" in the UI language.
    pub label: String,
    /// The same in English (matched too).
    pub english: String,
    pub shortcut: String,
    pub enabled: bool,
    /// Position in the recently used list.
    pub recent: Option<usize>,
}

/// Recently used commands shown first in the palette.
pub const PALETTE_RECENT: usize = 10;
/// Rows visible before the list scrolls.
const PALETTE_ROWS: f32 = 12.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// Highlight moved (themes preview live).
    Preview(String),
    Accept(String),
    Cancel,
    /// Quick Open prefix: switch to another provider with this query.
    Switch(PickerKind, String),
    /// Quick Open `:`: Go to Line with this text.
    GotoLine(String),
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
            PickerKind::Commands
            | PickerKind::Indentation
            | PickerKind::Symbols
            | PickerKind::QuickOpen
            | PickerKind::Compare
            | PickerKind::GitHistory
            | PickerKind::GitBranches
            | PickerKind::GitBranchName
            | PickerKind::GitCloneUrl => String::new(),
            PickerKind::TabSize => doc
                .map(|d| core.indent_style(d.id).tab_size.to_string())
                .unwrap_or_default(),
        };
        let (commands, context) = if kind == PickerKind::Commands {
            let ctx = core.context(false);
            (command_entries(core, &ctx), Some(ctx))
        } else {
            (Vec::new(), None)
        };
        let dynamic = match kind {
            PickerKind::Symbols => symbol_items(core),
            PickerKind::GitHistory => history_items(core),
            PickerKind::GitBranches => branch_items(core),
            PickerKind::GitCloneUrl => core
                .git
                .recent_urls
                .iter()
                .map(|u| PickerItem {
                    value: u.clone(),
                    label: u.clone(),
                    detail: t!("git:clone.recent"),
                })
                .collect(),
            _ => Vec::new(),
        };
        let restore = (kind == PickerKind::Symbols)
            .then(|| core.workbench.active_view())
            .flatten()
            .and_then(|v| core.workbench.view(v))
            .map(|v| (v.id, v.selection.clone(), v.scroll));
        let selected = Self::all_items(kind)
            .iter()
            .position(|i| i.value == current)
            .unwrap_or(0);
        let mut picker = Self {
            kind,
            query: String::new(),
            selected,
            focus_requested: false,
            commands,
            show_all: false,
            context,
            scroll_to_selected: true,
            dynamic,
            restore,
            quick: matches!(kind, PickerKind::QuickOpen | PickerKind::Compare)
                .then(|| crate::quick_open::Source::capture(core)),
            compare_hint: (kind == PickerKind::Compare).then(|| {
                match core.compare.pick_left.as_ref() {
                    Some(src) => t!(
                        "dialogs:compare.pickRight",
                        name = core.compare_source_name(src)
                    ),
                    None => t!("dialogs:compare.pickLeft"),
                }
            }),
            quick_rows: None,
            from_quick_open: false,
        };
        if kind == PickerKind::Commands {
            picker.selected = 0;
        }
        picker
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
            PickerKind::Commands
            | PickerKind::Symbols
            | PickerKind::QuickOpen
            | PickerKind::Compare
            | PickerKind::GitHistory
            | PickerKind::GitBranches
            | PickerKind::GitBranchName
            | PickerKind::GitCloneUrl => Vec::new(),
            PickerKind::Indentation => [
                ("spaces", "dialogs:picker.indentSpaces"),
                ("tabs", "dialogs:picker.indentTabs"),
                ("tabSize", "dialogs:picker.changeTabSize"),
                ("detect", "dialogs:picker.detectIndentation"),
                ("toSpaces", "dialogs:picker.convertToSpaces"),
                ("toTabs", "dialogs:picker.convertToTabs"),
            ]
            .into_iter()
            .map(|(v, k)| PickerItem {
                value: v.to_owned(),
                label: t!(k),
                detail: String::new(),
            })
            .collect(),
            PickerKind::TabSize => (1..=8)
                .map(|n| PickerItem {
                    value: n.to_string(),
                    label: n.to_string(),
                    detail: String::new(),
                })
                .collect(),
        }
    }

    /// Palette rows for the query: recently used first when empty, else
    /// best fuzzy match on the translated or English label.
    fn command_rows(&self) -> Vec<&CommandEntry> {
        let visible = self.commands.iter().filter(|c| self.show_all || c.enabled);
        let q = self.query.trim().trim_start_matches('>').trim();
        if q.is_empty() {
            let mut recent: Vec<&CommandEntry> =
                visible.clone().filter(|c| c.recent.is_some()).collect();
            recent.sort_by_key(|c| c.recent);
            let mut rest: Vec<&CommandEntry> = visible.filter(|c| c.recent.is_none()).collect();
            rest.sort_by_key(|c| c.label.to_lowercase());
            recent.extend(rest);
            return recent;
        }
        let mut scored: Vec<(i32, &CommandEntry)> = visible
            .filter_map(|c| {
                let a = notoir_core::fuzzy::fuzzy_match(q, &c.label).map(|m| m.score);
                let b = notoir_core::fuzzy::fuzzy_match(q, &c.english).map(|m| m.score);
                a.max(b)
                    .map(|s| (s + if c.recent.is_some() { 2 } else { 0 }, c))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
        scored.into_iter().map(|(_, c)| c).collect()
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
            PickerKind::Commands => {
                return CommandInvocation::new(value.to_owned(), InvocationSource::Palette);
            }
            PickerKind::Indentation => ("editor.changeIndentation", json!({ "action": value })),
            PickerKind::TabSize => (
                "editor.changeTabSize",
                json!({ "size": value.parse::<u64>().unwrap_or(4) }),
            ),
            PickerKind::Symbols => {
                let (a, b) = parse_range(value).unwrap_or((0, 0));
                ("navigate.revealRange", json!({ "start": a, "end": b }))
            }
            PickerKind::QuickOpen => ("quickOpen.open", quick_open_args(value)),
            PickerKind::Compare => ("compare.pick", json!({ "value": value })),
            PickerKind::GitHistory => ("git.historyPick", json!({ "rev": value })),
            PickerKind::GitBranches => ("git.branchPick", json!({ "value": value })),
            PickerKind::GitBranchName => ("git.createBranch", json!({ "name": value })),
            PickerKind::GitCloneUrl => ("git.clone", json!({ "url": value })),
        };
        CommandInvocation::new(id, InvocationSource::Palette).with_args(args)
    }

    /// Items matching the query (case-insensitive, label or detail).
    pub fn items(&self) -> Vec<PickerItem> {
        if self.kind == PickerKind::Commands {
            return self
                .command_rows()
                .into_iter()
                .map(|c| PickerItem {
                    value: c.id.clone(),
                    label: c.label.clone(),
                    detail: c.shortcut.clone(),
                })
                .collect();
        }
        if self.kind == PickerKind::QuickOpen {
            return self.quick_items();
        }
        if self.kind == PickerKind::Compare {
            return self.compare_items();
        }
        if matches!(
            self.kind,
            PickerKind::GitBranchName | PickerKind::GitCloneUrl
        ) {
            // Free text: the typed value first, then matching recent rows.
            let q = self.query.trim();
            let mut rows = Vec::new();
            if !q.is_empty() {
                rows.push(PickerItem {
                    value: q.to_owned(),
                    label: if self.kind == PickerKind::GitBranchName {
                        t!("git:branch.createNamed", name = q)
                    } else {
                        t!("git:clone.cloneUrl", url = q)
                    },
                    detail: String::new(),
                });
            }
            rows.extend(
                self.dynamic
                    .iter()
                    .filter(|i| i.value != q && i.value.contains(q))
                    .cloned(),
            );
            return rows;
        }
        if matches!(
            self.kind,
            PickerKind::Symbols | PickerKind::GitHistory | PickerKind::GitBranches
        ) {
            let q = self.query.trim().trim_start_matches('@').trim();
            if q.is_empty() {
                return self.dynamic.clone();
            }
            let mut scored: Vec<(i32, &PickerItem)> = self
                .dynamic
                .iter()
                .filter_map(|i| {
                    notoir_core::fuzzy::fuzzy_match(q, i.label.trim())
                        .or_else(|| notoir_core::fuzzy::fuzzy_match(q, &i.detail))
                        .map(|m| (m.score, i))
                })
                .collect();
            scored.sort_by_key(|s| std::cmp::Reverse(s.0));
            return scored.into_iter().map(|(_, i)| i.clone()).collect();
        }
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

    /// Quick Open rows (or the `?` help, `#` placeholder) for the query.
    fn quick_items(&self) -> Vec<PickerItem> {
        let q = self.query.trim_start();
        if q.starts_with('?') {
            return [
                (">", "dialogs:quickOpen.helpCommands"),
                (":", "dialogs:quickOpen.helpLine"),
                ("@", "dialogs:quickOpen.helpSymbol"),
                ("#", "dialogs:quickOpen.helpWorkspaceSymbols"),
            ]
            .into_iter()
            .map(|(p, k)| PickerItem {
                value: format!("?{p}"),
                label: t!(k),
                detail: p.to_owned(),
            })
            .collect();
        }
        if q.starts_with('#') {
            return vec![PickerItem {
                value: "#".into(),
                label: t!("dialogs:quickOpen.workspaceSymbolsLater"),
                detail: String::new(),
            }];
        }
        let (_, line) = crate::quick_open::split_line_suffix(q);
        let Some((_, rows)) = &self.quick_rows else {
            return Vec::new();
        };
        rows.iter()
            .map(|r| {
                let target = match &r.target {
                    crate::quick_open::Target::Path(p) => format!("p:{}", p.display()),
                    crate::quick_open::Target::Doc(d) => format!("d:{}", d.0),
                };
                let suffix = line.map_or(String::new(), |(l, c)| format!("\n{l}\n{c}"));
                PickerItem {
                    value: format!("{target}{suffix}"),
                    label: r.name.clone(),
                    detail: r.dir.clone(),
                }
            })
            .collect()
    }

    /// Compare rows: the clipboard, then Quick Open's files.
    fn compare_items(&self) -> Vec<PickerItem> {
        let q = self.query.trim();
        let clip = t!("compare:clipboard");
        let mut out = Vec::new();
        if q.is_empty() || notoir_core::fuzzy::fuzzy_match(q, &clip).is_some() {
            out.push(PickerItem {
                value: "clipboard".into(),
                label: clip,
                detail: String::new(),
            });
        }
        if let Some((_, rows)) = &self.quick_rows {
            out.extend(rows.iter().map(|r| PickerItem {
                value: match &r.target {
                    crate::quick_open::Target::Path(p) => format!("p:{}", p.display()),
                    crate::quick_open::Target::Doc(d) => format!("d:{}", d.0),
                },
                label: r.name.clone(),
                detail: r.dir.clone(),
            }));
        }
        out
    }

    /// Recomputes Quick Open rows when the query changed.
    fn refresh_quick(&mut self) {
        let Some(src) = &self.quick else { return };
        let (q, _) = crate::quick_open::split_line_suffix(self.query.trim());
        if self.quick_rows.as_ref().is_some_and(|(k, _)| k == q) {
            return;
        }
        let rows = src.rows(q);
        self.quick_rows = Some((q.to_owned(), rows));
    }

    /// Prefixes typed into Quick Open switch providers; deleting the prefix
    /// in a provider reached from Quick Open goes back.
    fn prefix_switch(&self) -> Option<PickerEvent> {
        let q = self.query.as_str();
        match self.kind {
            PickerKind::QuickOpen => match q.chars().next() {
                Some('>') => Some(PickerEvent::Switch(PickerKind::Commands, q.to_owned())),
                Some('@') => Some(PickerEvent::Switch(PickerKind::Symbols, q.to_owned())),
                Some(':') => Some(PickerEvent::GotoLine(q[1..].to_owned())),
                _ => None,
            },
            PickerKind::Commands if self.from_quick_open && !q.starts_with('>') => {
                Some(PickerEvent::Switch(PickerKind::QuickOpen, q.to_owned()))
            }
            PickerKind::Symbols if self.from_quick_open && !q.starts_with('@') => {
                Some(PickerEvent::Switch(PickerKind::QuickOpen, q.to_owned()))
            }
            _ => None,
        }
    }

    pub fn set_query(&mut self, q: String) {
        self.query = q;
        self.selected = 0;
        self.scroll_to_selected = true;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        kit: &mut Kit,
        top_offset: f32,
    ) -> Option<PickerEvent> {
        let mut event = None;
        self.refresh_quick();
        let side = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Enter));
        if side && self.kind == PickerKind::QuickOpen {
            return self
                .items()
                .get(self.selected)
                .filter(|i| i.value.starts_with("p:") || i.value.starts_with("d:"))
                .map(|i| PickerEvent::Accept(format!("{}\nside", i.value)));
        }
        let items_before = self.items();
        let (down, up, enter, esc, toggle_all) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::P),
            )
        });
        if toggle_all && self.kind == PickerKind::Commands {
            self.show_all = !self.show_all;
            self.selected = 0;
        }
        if down || up {
            self.scroll_to_selected = true;
        }
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
        if enter && self.kind != PickerKind::Commands {
            let value = items_before.get(self.selected).map(|i| i.value.clone());
            return match value {
                // `?` help rows insert their prefix; `#` does nothing yet.
                Some(v) if self.kind == PickerKind::QuickOpen && v.starts_with('?') => {
                    self.set_query(v[1..].to_owned());
                    self.prefix_switch()
                }
                Some(v) if self.kind == PickerKind::QuickOpen && v == "#" => None,
                Some(v) => Some(PickerEvent::Accept(v)),
                None => None,
            };
        }

        let title = match self.kind {
            PickerKind::Theme => t!("dialogs:picker.theme"),
            PickerKind::Language => t!("dialogs:picker.language"),
            PickerKind::LineEnding => t!("dialogs:picker.lineEnding"),
            PickerKind::EncodingAction => t!("dialogs:picker.encodingAction"),
            PickerKind::Encoding { .. } => t!("dialogs:picker.encoding"),
            PickerKind::LanguageMode => t!("dialogs:picker.languageMode"),
            PickerKind::Commands => String::new(),
            PickerKind::Indentation => t!("dialogs:picker.indentation"),
            PickerKind::TabSize => t!("dialogs:picker.tabSize"),
            PickerKind::Symbols => t!("dialogs:picker.symbols"),
            PickerKind::QuickOpen => String::new(),
            PickerKind::Compare => t!("dialogs:compare.title"),
            PickerKind::GitHistory => t!("dialogs:git.history"),
            PickerKind::GitBranches => t!("git:branch.pickTitle"),
            PickerKind::GitBranchName => t!("git:branch.nameTitle"),
            PickerKind::GitCloneUrl => t!("git:clone.title"),
        };
        if self.kind == PickerKind::Commands && enter {
            // Disabled rows (shown with "show all") do not run.
            let rows = self.command_rows();
            return rows
                .get(self.selected)
                .filter(|c| c.enabled)
                .map(|c| PickerEvent::Accept(c.id.clone()));
        }
        let width = 560.0_f32.min(ctx.content_rect().width() - 32.0);
        let area = egui::Area::new(egui::Id::new("picker"))
            .anchor(Align2::CENTER_TOP, [0.0, top_offset + 8.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                overlay_frame(kit).show(ui, |ui| {
                    ui.set_width(width);
                    if !title.is_empty() {
                        section_label(ui, kit, &title);
                    }
                    let hint = match self.kind {
                        PickerKind::Commands => t!("dialogs:picker.commands"),
                        PickerKind::QuickOpen => t!("dialogs:quickOpen.hint"),
                        PickerKind::Compare => self
                            .compare_hint
                            .clone()
                            .unwrap_or_else(|| t!("dialogs:picker.filter")),
                        PickerKind::GitBranchName => t!("git:branch.nameHint"),
                        PickerKind::GitCloneUrl => t!("git:clone.urlHint"),
                        _ => t!("dialogs:picker.filter"),
                    };
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    );
                    if !self.focus_requested {
                        edit.request_focus();
                        self.focus_requested = true;
                    }
                    if edit.changed() {
                        self.selected = 0;
                        self.scroll_to_selected = true;
                        if let Some(e) = self.prefix_switch() {
                            event = Some(e);
                        }
                        self.refresh_quick();
                    }
                    if self.quick.as_ref().is_some_and(|q| q.indexing) {
                        ui.label(
                            RichText::new(t!("dialogs:quickOpen.indexing"))
                                .size(11.0)
                                .color(kit.color(Tok::TextTertiary)),
                        );
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
                    let palette = self.kind == PickerKind::Commands;
                    let rows: Vec<(bool, bool)> = if palette {
                        self.command_rows()
                            .iter()
                            .map(|c| (c.enabled, c.recent.is_some()))
                            .collect()
                    } else {
                        vec![(true, false); items.len()]
                    };
                    let show_recent = palette && self.query.trim().is_empty();
                    let scroll_to = std::mem::take(&mut self.scroll_to_selected);
                    let mut hovered = None;
                    let mut clicked = None;
                    egui::ScrollArea::vertical()
                        .max_height(PALETTE_ROWS * 26.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for (i, item) in items.iter().enumerate() {
                                let (enabled, recent) = rows[i];
                                if show_recent && i == 0 && recent {
                                    section_label(ui, kit, &t!("dialogs:picker.recentlyUsed"));
                                }
                                if show_recent && i > 0 && rows[i - 1].1 && !recent {
                                    ui.separator();
                                }
                                let selected = i == self.selected;
                                let color = if enabled {
                                    kit.color(Tok::TextPrimary)
                                } else {
                                    kit.color(Tok::TextDisabled)
                                };
                                // Quick Open: matched characters of the name in bold.
                                let marks = self
                                    .quick_rows
                                    .as_ref()
                                    .filter(|_| self.kind == PickerKind::QuickOpen)
                                    .and_then(|(_, r)| r.get(i))
                                    .map(|r| r.name_marks.clone())
                                    .unwrap_or_default();
                                let label: egui::WidgetText = if marks.is_empty() {
                                    RichText::new(&item.label).size(13.0).color(color).into()
                                } else {
                                    let mut job = egui::text::LayoutJob::default();
                                    for (ci, ch) in item.label.chars().enumerate() {
                                        let font = if marks.contains(&ci) {
                                            fonts::extrabold(13.0)
                                        } else {
                                            egui::FontId::proportional(13.0)
                                        };
                                        job.append(
                                            &ch.to_string(),
                                            0.0,
                                            egui::TextFormat::simple(font, color),
                                        );
                                    }
                                    job.into()
                                };
                                let resp = ui.add(
                                    egui::Button::selectable(selected, label)
                                        .right_text(
                                            RichText::new(&item.detail)
                                                .font(fonts::mono(12.0))
                                                .color(kit.color(Tok::TextTertiary)),
                                        )
                                        .min_size(egui::vec2(width, 26.0)),
                                );
                                if selected && scroll_to {
                                    resp.scroll_to_me(None);
                                }
                                if resp.hovered()
                                    && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                                {
                                    hovered = Some(i);
                                }
                                if resp.clicked() && enabled {
                                    clicked = Some(item.value.clone());
                                }
                            }
                        });
                    if let Some(i) = hovered {
                        self.selected = i;
                    }
                    if let Some(v) = clicked {
                        event = Some(PickerEvent::Accept(v));
                    }
                    if self.kind == PickerKind::QuickOpen
                        && let Some(PickerEvent::Accept(v)) = &event
                        && (v.starts_with('?') || v == "#")
                    {
                        let v = v.clone();
                        event = None;
                        if let Some(p) = v.strip_prefix('?') {
                            self.set_query(p.to_owned());
                            event = self.prefix_switch();
                        }
                    }
                });
            });
        if event.is_none() && area.response.clicked_elsewhere() {
            return Some(PickerEvent::Cancel);
        }
        if event.is_none()
            && self.selected != before
            && matches!(self.kind, PickerKind::Theme | PickerKind::Symbols)
        {
            event = self
                .items()
                .get(self.selected)
                .map(|i| PickerEvent::Preview(i.value.clone()));
        }
        event
    }
}

/// Quick Open row value → `quickOpen.open` args.
fn quick_open_args(value: &str) -> serde_json::Value {
    let mut parts = value.split('\n');
    let target = parts.next().unwrap_or("");
    let line = parts.next().and_then(|l| l.parse::<u64>().ok());
    let col = parts.next().and_then(|c| c.parse::<u64>().ok());
    let side = value.ends_with("\nside");
    let mut args = serde_json::json!({ "side": side });
    if let Some(p) = target.strip_prefix("p:") {
        args["path"] = p.into();
    } else if let Some(d) = target
        .strip_prefix("d:")
        .and_then(|d| d.parse::<u64>().ok())
    {
        args["doc"] = d.into();
    }
    if let Some(l) = line {
        args["line"] = l.into();
        args["col"] = col.unwrap_or(1).into();
    }
    args
}

/// `"start:end"` of a symbol row.
pub fn parse_range(value: &str) -> Option<(usize, usize)> {
    let (a, b) = value.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Go to Symbol rows: the active document's outline, indented by depth.
fn symbol_items(core: &AppCore) -> Vec<PickerItem> {
    let Some(doc) = core.workbench.active_document() else {
        return Vec::new();
    };
    let Some(s) = core.structure.docs.get(&doc) else {
        return Vec::new();
    };
    let rope = core
        .workbench
        .document(doc)
        .map(|d| d.buffer.rope().clone());
    notoir_editor::outline::flatten(&s.symbols)
        .into_iter()
        .map(|(depth, sym)| {
            let line = rope.as_ref().map_or(0, |r| {
                r.char_to_line(sym.selection_range.0.min(r.len_chars())) + 1
            });
            PickerItem {
                value: format!("{}:{}", sym.selection_range.0, sym.selection_range.0),
                label: format!("{}{}", "  ".repeat(depth), sym.name),
                detail: t!("dialogs:picker.symbolLine", line = line),
            }
        })
        .collect()
}

/// Palette rows: commands listed in the palette, enabled per the captured
/// context, with shortcuts and recent use.
fn command_entries(core: &AppCore, ctx: &notoir_core::when::ContextSnapshot) -> Vec<CommandEntry> {
    let recent: Vec<String> = core
        .state_db
        .as_ref()
        .and_then(|db| db.palette_recent(PALETTE_RECENT).ok())
        .unwrap_or_default();
    core.commands
        .iter()
        .filter(|c| c.palette)
        .map(|c| {
            let label = format!("{}: {}", t!(c.category.title_key()), t!(&c.title_key));
            let english = format!(
                "{}: {}",
                i18n::t_in(Language::En, c.category.title_key()),
                i18n::t_in(Language::En, &c.title_key)
            );
            let shortcut = core
                .keymap
                .chords_for(&c.id)
                .first()
                .map(|k| k.display(core.platform))
                .unwrap_or_default();
            CommandEntry {
                id: c.id.as_str().to_owned(),
                label,
                english,
                shortcut,
                enabled: c.enablement.eval(ctx),
                recent: recent.iter().position(|r| r == c.id.as_str()),
            }
        })
        .collect()
}

/// Go to Line input (spec 11 §Go to Line): previews while typing, Enter
/// keeps, Esc restores.
pub fn goto_line(ctx: &egui::Context, kit: &mut Kit, core: &mut AppCore, top_offset: f32) {
    let Some(g) = core.find.goto.clone() else {
        return;
    };
    let lines = core
        .workbench
        .view(g.view)
        .and_then(|v| core.workbench.document(v.document_id))
        .map_or(1, |d| d.buffer.rope().len_lines());
    let (enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
            i.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if esc {
        core.close_goto(false);
        return;
    }
    if enter {
        core.close_goto(true);
        return;
    }
    let mut text = g.text.clone();
    let width = 420.0_f32.min(ctx.content_rect().width() - 32.0);
    let area = egui::Area::new(egui::Id::new("goto-line"))
        .anchor(Align2::CENTER_TOP, [0.0, top_offset + 8.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            overlay_frame(kit).show(ui, |ui| {
                ui.set_width(width);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(egui::Id::new("goto-line-input"))
                        .hint_text(t!("dialogs:gotoLine.hint", lines = lines))
                        .desired_width(f32::INFINITY),
                );
                if g.focus {
                    resp.request_focus();
                }
            });
        });
    if g.focus
        && let Some(x) = core.find.goto.as_mut()
    {
        x.focus = false;
    }
    if text != g.text {
        core.goto_preview(text);
    }
    if area.response.clicked_elsewhere() {
        core.close_goto(true);
    }
}

/// Branch picker rows: Create Branch… / Create Branch From… (when
/// switching), then local branches (newest first) and remote branches.
fn branch_items(core: &AppCore) -> Vec<PickerItem> {
    let mut rows = Vec::new();
    if core.git.branch_pick == crate::git_write::BranchPick::Switch {
        rows.push(PickerItem {
            value: ":create".into(),
            label: format!("+ {}", t!("git:branch.create")),
            detail: String::new(),
        });
        rows.push(PickerItem {
            value: ":createFrom".into(),
            label: format!("+ {}", t!("git:branch.createFrom")),
            detail: String::new(),
        });
    }
    rows.extend(core.git.branches.iter().map(|b| PickerItem {
        value: b.name.clone(),
        label: b.name.clone(),
        detail: if b.current {
            t!("git:branch.current")
        } else if b.remote {
            t!("git:branch.remote")
        } else {
            b.upstream.clone().unwrap_or_default()
        },
    }));
    rows
}

/// File History rows: summary, then short SHA · author · date.
fn history_items(core: &AppCore) -> Vec<PickerItem> {
    let Some(h) = &core.git.history else {
        return Vec::new();
    };
    h.commits
        .iter()
        .map(|c| PickerItem {
            value: c.id.clone(),
            label: c.summary.clone(),
            detail: format!("{} \u{b7} {} \u{b7} {}", c.short, c.author, ymd(c.time)),
        })
        .collect()
}

/// `YYYY-MM-DD` (UTC) of a Unix time.
pub fn ymd(secs: i64) -> String {
    // Days to civil date (Howard Hinnant's algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod date_tests {
    #[test]
    fn civil_dates() {
        assert_eq!(super::ymd(0), "1970-01-01");
        assert_eq!(super::ymd(1_760_486_400), "2025-10-15");
        assert_eq!(super::ymd(951_782_400), "2000-02-29");
    }
}
