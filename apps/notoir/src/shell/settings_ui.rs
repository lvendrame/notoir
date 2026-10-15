//! The Settings tab (docs/specs/20 §Settings UI): search, User/Workspace scope,
//! categories, one control per setting with modified/overridden markers and a
//! per-row menu, unknown settings, and the Keyboard Shortcuts editor.

use egui::{Align, Layout, RichText, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::keymap::{BindingSource, KeyChord};
use notoir_core::settings::{SettingCategory, SettingDef, SettingType, SettingValue};
use serde_json::json;

use super::{Actions, Kit};
use crate::core::{AppCore, Effect};
use crate::i18n::{self, Language};
use crate::settings_tab::{CATEGORIES, category_key};
use crate::t;
use crate::theme::Tok;

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

/// Title of a setting: translated, falling back to the last key segment.
pub fn setting_title(key: &str) -> String {
    let k = format!("settings:{key}");
    let s = t!(&k);
    if s == k { humanize(key) } else { s }
}

fn humanize(key: &str) -> String {
    let last = key.rsplit('.').next().unwrap_or(key);
    let mut out = String::new();
    for (i, c) in last.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn value_text(v: &SettingValue) -> String {
    match v {
        SettingValue::Bool(b) => b.to_string(),
        SettingValue::Int(i) => i.to_string(),
        SettingValue::Float(f) => format!("{f}"),
        SettingValue::Str(s) => format!("\"{s}\""),
        SettingValue::List(l) => format!(
            "[{}]",
            l.iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Whether a setting matches the search box (title in both languages, key,
/// tags, `@modified`).
fn matches(core: &AppCore, def: &SettingDef, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let mut words: Vec<&str> = Vec::new();
    for w in q.split_whitespace() {
        if w == "@modified" {
            let modified = core.user_settings.values.contains_key(def.key)
                || core
                    .workspace
                    .as_ref()
                    .and_then(|w| w.settings.as_ref())
                    .is_some_and(|l| l.values.contains_key(def.key));
            if !modified {
                return false;
            }
        } else {
            words.push(w);
        }
    }
    let hay = format!(
        "{} {} {} {}",
        setting_title(def.key).to_lowercase(),
        i18n::t_in(Language::En, &format!("settings:{}", def.key)).to_lowercase(),
        def.key.to_lowercase(),
        def.tags.join(" ").to_lowercase()
    );
    words.iter().all(|w| hay.contains(w))
}

pub fn show(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(20.0, 12.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    // Header: search, scope, open file.
    ui.horizontal(|ui| {
        let mut search = core.settings_ui.search.clone();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut search)
                .id(egui::Id::new("settings-search"))
                .hint_text(t!("settings:ui.search"))
                .desired_width(320.0),
        );
        if core.settings_ui.focus_search {
            resp.request_focus();
            core.settings_ui.focus_search = false;
        }
        core.settings_ui.search = search;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("settings:ui.openFile")).clicked() {
                if core.settings_ui.workspace_scope
                    && let Some(ws) = &core.workspace
                {
                    let path = ws.root.join(".notoir").join("settings.toml");
                    core.open_config_file(path, "# Workspace settings\n");
                } else {
                    cmd(actions, "preferences.openSettingsFile", json!({}));
                }
            }
            let has_ws = core.workspace.is_some();
            if !has_ws {
                core.settings_ui.workspace_scope = false;
            }
            ui.add_enabled_ui(has_ws, |ui| {
                ui.selectable_value(
                    &mut core.settings_ui.workspace_scope,
                    true,
                    t!("settings:ui.workspace"),
                );
            });
            ui.selectable_value(
                &mut core.settings_ui.workspace_scope,
                false,
                t!("settings:ui.user"),
            );
        });
    });
    ui.add_space(8.0);
    ui.separator();
    let body = ui.available_rect_before_wrap();
    let nav_w = 170.0;
    let nav = egui::Rect::from_min_size(body.min, Vec2::new(nav_w, body.height()));
    let list =
        egui::Rect::from_min_max(egui::pos2(body.left() + nav_w + 16.0, body.top()), body.max);
    // Category list.
    let mut nav_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(nav)
            .layout(Layout::top_down(Align::Min)),
    );
    for &c in CATEGORIES {
        let selected = core.settings_ui.category == Some(c)
            || (core.settings_ui.category.is_none() && c == SettingCategory::General);
        if nav_ui
            .add(
                egui::Button::selectable(selected, RichText::new(t!(category_key(c))).size(13.0))
                    .min_size(Vec2::new(nav_w - 8.0, 26.0)),
            )
            .clicked()
        {
            core.settings_ui.category = Some(c);
            core.settings_ui.search.clear();
        }
    }
    let mut list_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(list)
            .layout(Layout::top_down(Align::Min)),
    );
    let searching = !core.settings_ui.search.trim().is_empty();
    let category = core
        .settings_ui
        .category
        .unwrap_or(SettingCategory::General);
    if !searching && category == SettingCategory::Keyboard {
        shortcuts(&mut list_ui, kit, core, actions);
        return;
    }
    let defs: Vec<SettingDef> = core
        .schema
        .iter()
        .filter(|d| {
            if searching {
                matches(core, d, &core.settings_ui.search)
            } else {
                d.category == category
            }
        })
        .cloned()
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("settings-list")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            if !searching {
                ui.label(
                    RichText::new(t!(category_key(category)))
                        .font(crate::fonts::semibold(18.0))
                        .color(kit.color(Tok::TextPrimary)),
                );
                ui.add_space(8.0);
            }
            if defs.is_empty() {
                ui.label(
                    RichText::new(t!("settings:ui.noResults")).color(kit.color(Tok::TextTertiary)),
                );
            }
            for def in &defs {
                row(ui, kit, core, def, actions);
                ui.add_space(10.0);
            }
            if !searching && category == SettingCategory::Advanced {
                unknown(ui, kit, core);
            }
        });
}

fn row(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut AppCore,
    def: &SettingDef,
    actions: &mut Actions,
) {
    let key = def.key;
    let in_user = core.user_settings.values.contains_key(key);
    let in_ws = core
        .workspace
        .as_ref()
        .and_then(|w| w.settings.as_ref())
        .is_some_and(|l| l.values.contains_key(key));
    let ws_scope = core.settings_ui.workspace_scope;
    let modified_here = if ws_scope { in_ws } else { in_user };
    let effective = core
        .settings
        .get(key, None)
        .cloned()
        .unwrap_or_else(|| def.default.clone());
    let shown = core.scope_value(key).unwrap_or_else(|| effective.clone());
    let start = ui.cursor().min;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(setting_title(key))
                .size(14.0)
                .color(kit.color(Tok::TextPrimary)),
        );
        ui.label(
            RichText::new(key)
                .font(crate::fonts::mono(11.0))
                .color(kit.color(Tok::TextTertiary)),
        );
        if modified_here {
            let k = if ws_scope {
                "settings:ui.modifiedWorkspace"
            } else {
                "settings:ui.modifiedUser"
            };
            ui.label(
                RichText::new(t!(k))
                    .size(11.0)
                    .color(kit.color(Tok::Accent)),
            );
        }
        if !ws_scope && in_ws && def.scopes.workspace {
            ui.label(
                RichText::new(t!("settings:ui.overriddenWorkspace"))
                    .size(11.0)
                    .color(kit.color(Tok::Warning)),
            );
        }
        if def.restricted && ws_scope {
            ui.label(
                RichText::new(t!("settings:ui.restricted"))
                    .size(11.0)
                    .color(kit.color(Tok::Warning)),
            );
        }
        let gear = super::icon_button_sized(
            ui,
            kit,
            crate::icons::Icon::Ellipsis,
            &t!("settings:ui.more"),
            false,
            true,
            20.0,
        );
        egui::Popup::menu(&gear).show(|ui| {
            if ui
                .add_enabled(modified_here, egui::Button::new(t!("settings:ui.reset")))
                .clicked()
            {
                core.set_setting_in_scope(key, None);
                ui.close();
            }
            if ui.button(t!("settings:ui.copyId")).clicked() {
                core.effects.push(Effect::CopyText(key.to_owned()));
                ui.close();
            }
            if ui.button(t!("settings:ui.copyToml")).clicked() {
                core.effects
                    .push(Effect::CopyText(format!("{key} = {}", value_text(&shown))));
                ui.close();
            }
        });
    });
    let scope_ok = !ws_scope || def.scopes.workspace;
    let new_value = ui
        .add_enabled_ui(scope_ok, |ui| control(ui, core, def, &shown))
        .inner;
    if let Some(v) = new_value {
        match key {
            "workbench.theme" => {
                if let SettingValue::Str(s) = &v {
                    cmd(actions, "preferences.changeTheme", json!({ "theme": s }));
                }
            }
            "language" if !ws_scope => {
                if let SettingValue::Str(s) = &v {
                    cmd(
                        actions,
                        "preferences.changeLanguage",
                        json!({ "language": s }),
                    );
                }
            }
            _ => core.set_setting_in_scope(key, Some(v)),
        }
    }
    if modified_here {
        let end = ui.cursor().min;
        ui.painter().line_segment(
            [
                egui::pos2(start.x - 8.0, start.y),
                egui::pos2(start.x - 8.0, end.y),
            ],
            Stroke::new(2.0, kit.color(Tok::Accent)),
        );
    }
}

/// The control for a setting; returns a new value when the user changed it.
fn control(
    ui: &mut egui::Ui,
    core: &AppCore,
    def: &SettingDef,
    value: &SettingValue,
) -> Option<SettingValue> {
    let id = egui::Id::new(("setting", def.key));
    match (&def.ty, value) {
        (SettingType::Bool, SettingValue::Bool(b)) => {
            let mut v = *b;
            ui.checkbox(&mut v, "")
                .changed()
                .then_some(SettingValue::Bool(v))
        }
        (SettingType::Int { min, max }, SettingValue::Int(i)) => {
            let mut v = *i;
            ui.add(egui::DragValue::new(&mut v).range(*min..=*max))
                .changed()
                .then_some(SettingValue::Int(v))
        }
        (SettingType::Float { min, max }, SettingValue::Float(f)) => {
            let mut v = *f;
            ui.add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.05))
                .changed()
                .then_some(SettingValue::Float(v))
        }
        (SettingType::Enum(options), SettingValue::Str(s)) => {
            let mut v = s.clone();
            egui::ComboBox::from_id_salt(id)
                .selected_text(&v)
                .show_ui(ui, |ui| {
                    for o in *options {
                        ui.selectable_value(&mut v, (*o).to_owned(), *o);
                    }
                });
            (v != *s).then_some(SettingValue::Str(v))
        }
        (SettingType::Str, SettingValue::Str(s)) if def.key == "language" => {
            let mut v = s.clone();
            let shown = Language::from_code(&v).map_or_else(
                || t!("settings:ui.languageAuto"),
                |l| l.native_label().to_owned(),
            );
            egui::ComboBox::from_id_salt(id)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for l in Language::ALL {
                        ui.selectable_value(&mut v, l.code().to_owned(), l.native_label());
                    }
                });
            (v != *s).then_some(SettingValue::Str(v))
        }
        (SettingType::Str, SettingValue::Str(s)) if def.key == "workbench.theme" => {
            let mut v = s.clone();
            egui::ComboBox::from_id_salt(id)
                .selected_text(&v)
                .show_ui(ui, |ui| {
                    for t in crate::theme::ThemeId::ALL {
                        ui.selectable_value(&mut v, t.id().to_owned(), t.load().name);
                    }
                    ui.selectable_value(&mut v, "auto".to_owned(), t!("dialogs:picker.themeAuto"));
                });
            (v != *s).then_some(SettingValue::Str(v))
        }
        (SettingType::Str, SettingValue::Str(s)) => text_control(ui, id, s).map(SettingValue::Str),
        (SettingType::StrList, SettingValue::List(l)) => {
            let joined = l.join(", ");
            text_control(ui, id, &joined).map(|t| {
                SettingValue::List(
                    t.split(',')
                        .map(|x| x.trim().to_owned())
                        .filter(|x| !x.is_empty())
                        .collect(),
                )
            })
        }
        _ => {
            let _ = core;
            ui.label(value_text(value));
            None
        }
    }
}

/// A text field that applies on Enter or when it loses focus.
fn text_control(ui: &mut egui::Ui, id: egui::Id, current: &str) -> Option<String> {
    let mut buf = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_owned());
    let resp = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .id(id.with("edit"))
            .desired_width(320.0),
    );
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
        None
    } else if resp.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        (buf != current).then_some(buf)
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
        None
    }
}

/// Keys in the user file that no setting declares (kept, never deleted).
fn unknown(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore) {
    let unknown: Vec<(&String, &SettingValue)> = core
        .user_settings
        .values
        .iter()
        .filter(|(k, _)| core.schema.get(k).is_none())
        .collect();
    if unknown.is_empty() {
        return;
    }
    ui.add_space(12.0);
    super::section_label(ui, kit, &t!("settings:ui.unknown"));
    for (k, v) in unknown {
        ui.label(
            RichText::new(format!("{k} = {}", value_text(v)))
                .font(crate::fonts::mono(12.0))
                .color(kit.color(Tok::TextSecondary)),
        );
    }
}

/// (command id, label, English label, [(keys, when, source)]).
type ShortcutRow = (String, String, String, Vec<(String, String, BindingSource)>);

fn source_label(s: BindingSource) -> String {
    match s {
        BindingSource::Default => t!("settings:kb.default"),
        BindingSource::Plugin => t!("settings:kb.plugin"),
        BindingSource::User => t!("settings:kb.user"),
    }
}

/// Keyboard Shortcuts editor: Command | Keybinding | When | Source, with
/// chord recording and conflict warnings.
fn shortcuts(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    ui.label(
        RichText::new(t!("settings:category.keyboard"))
            .font(crate::fonts::semibold(18.0))
            .color(kit.color(Tok::TextPrimary)),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut q = core.settings_ui.kb_search.clone();
        ui.add(
            egui::TextEdit::singleline(&mut q)
                .hint_text(t!("settings:kb.search"))
                .desired_width(320.0),
        );
        core.settings_ui.kb_search = q;
        if ui
            .button(t!("commands:preferences.openKeybindingsFile"))
            .clicked()
        {
            cmd(actions, "preferences.openKeybindingsFile", json!({}));
        }
    });
    record_keys(ui, core);
    ui.add_space(6.0);
    let platform = core.platform;
    let conflicts: Vec<String> = core
        .keymap
        .conflicts()
        .iter()
        .flat_map(|(a, b)| [a.command.as_str().to_owned(), b.command.as_str().to_owned()])
        .collect();
    let q = core.settings_ui.kb_search.trim().to_lowercase();
    let mut rows: Vec<ShortcutRow> = core
        .commands
        .iter()
        .filter(|c| c.palette)
        .map(|c| {
            let label = format!("{}: {}", t!(c.category.title_key()), t!(&c.title_key));
            let binds: Vec<(String, String, BindingSource)> = core
                .keymap
                .bindings()
                .iter()
                .filter(|b| b.command == c.id)
                .map(|b| (b.chord.display(platform), b.when.to_string(), b.source))
                .collect();
            (
                c.id.as_str().to_owned(),
                label,
                i18n::t_in(Language::En, &c.title_key),
                binds,
            )
        })
        .filter(|(id, label, en, binds)| {
            q.is_empty()
                || label.to_lowercase().contains(&q)
                || en.to_lowercase().contains(&q)
                || id.to_lowercase().contains(&q)
                || binds.iter().any(|(k, _, _)| k.to_lowercase().contains(&q))
        })
        .collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    egui::ScrollArea::vertical()
        .id_salt("shortcuts")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("shortcut-grid")
                .num_columns(5)
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for h in [
                        "settings:kb.command",
                        "settings:kb.keybinding",
                        "settings:kb.when",
                        "settings:kb.source",
                    ] {
                        ui.label(RichText::new(t!(h)).size(12.0).strong());
                    }
                    ui.label("");
                    ui.end_row();
                    for (id, label, _, binds) in rows {
                        let recording = core
                            .settings_ui
                            .recording
                            .as_ref()
                            .is_some_and(|(c, _)| *c == id);
                        let warn = if conflicts.contains(&id) { "⚠ " } else { "" };
                        let cell = |ui: &mut egui::Ui, w: f32, text: RichText| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(w, 20.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_width(w);
                                    ui.add(egui::Label::new(text).truncate());
                                },
                            );
                        };
                        cell(
                            ui,
                            250.0,
                            RichText::new(format!("{warn}{label}")).size(13.0),
                        );
                        let keys = if recording {
                            let typed = core.settings_ui.recording.as_ref().map(|(_, s)| {
                                s.iter()
                                    .map(|k| k.display(platform))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            });
                            format!("{} …", typed.unwrap_or_default())
                        } else {
                            binds
                                .iter()
                                .map(|b| b.0.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        cell(
                            ui,
                            140.0,
                            RichText::new(keys).font(crate::fonts::mono(12.0)),
                        );
                        cell(
                            ui,
                            100.0,
                            RichText::new(binds.first().map(|b| b.1.clone()).unwrap_or_default())
                                .font(crate::fonts::mono(11.0))
                                .color(kit.color(Tok::TextTertiary)),
                        );
                        cell(
                            ui,
                            60.0,
                            RichText::new(
                                binds.first().map(|b| source_label(b.2)).unwrap_or_default(),
                            )
                            .size(12.0)
                            .color(kit.color(Tok::TextSecondary)),
                        );
                        ui.horizontal(|ui| {
                            if recording {
                                if ui.button(t!("common:cancel")).clicked() {
                                    core.settings_ui.recording = None;
                                }
                            } else if ui
                                .button(t!("settings:kb.change"))
                                .on_hover_text(t!("settings:kb.changeHint"))
                                .clicked()
                            {
                                core.settings_ui.recording = Some((id.clone(), Vec::new()));
                            }
                            if !binds.is_empty()
                                && !recording
                                && ui.button(t!("settings:kb.remove")).clicked()
                            {
                                core.write_keybinding(None, &format!("-{id}"));
                            }
                        });
                        ui.end_row();
                    }
                });
        });
}

/// While recording: the next one or two keystrokes become the binding
/// (Enter confirms a single stroke early, Esc cancels).
fn record_keys(ui: &mut egui::Ui, core: &mut AppCore) {
    let Some((command, strokes)) = core.settings_ui.recording.clone() else {
        return;
    };
    let events: Vec<(egui::Key, egui::Modifiers)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    });
    let mut strokes = strokes;
    let mut done = false;
    for (key, mods) in events {
        ui.input_mut(|i| i.consume_key(mods, key));
        if key == egui::Key::Escape && mods.is_none() {
            core.settings_ui.recording = None;
            return;
        }
        if key == egui::Key::Enter && mods.is_none() && !strokes.is_empty() {
            done = true;
            break;
        }
        if let Some(s) = crate::input::keystroke(key, mods, core.platform) {
            strokes.push(s);
        }
        if strokes.len() == 2 {
            done = true;
            break;
        }
    }
    if done {
        if let Some(chord) = KeyChord::from_strokes(strokes) {
            core.write_keybinding(Some(&chord), &command);
        }
        core.settings_ui.recording = None;
    } else {
        core.settings_ui.recording = Some((command, strokes));
    }
}
