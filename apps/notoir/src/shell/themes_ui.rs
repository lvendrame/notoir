//! Native theme manager: drafts, typed controls, file rules and real editor preview.
use egui::{RichText, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::settings::{SettingType, SettingValue};
use serde_json::{Value, json};

use super::{Actions, Kit};
use crate::core::AppCore;
use crate::t;
use crate::theme::{Theme, ThemeId, Tok};
use crate::theme_manager::{self, Manager};

fn command(actions: &mut Actions, args: Value) {
    actions
        .push(CommandInvocation::new("themes.action", InvocationSource::Internal).with_args(args));
}
fn button(ui: &mut egui::Ui, actions: &mut Actions, label: &str, action: &str) {
    if ui.button(label).clicked() {
        command(actions, json!({"action":action}));
    }
}
fn edit(actions: &mut Actions, path: Vec<String>, value: Value) {
    command(
        actions,
        json!({"action":"edit", "path":path, "value":value}),
    );
}
fn root(manager: &Manager, section: &str) -> Vec<String> {
    match &manager.rule {
        Some((kind, key)) => vec![
            "appearance".into(),
            kind.clone(),
            key.clone(),
            section.into(),
        ],
        None => vec!["appearance".into(), section.into()],
    }
}

pub fn show(ui: &mut egui::Ui, kit: &mut Kit, core: &mut AppCore, actions: &mut Actions) {
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, kit.color(Tok::BgEditor));
    let mut manager = std::mem::take(&mut core.theme_manager);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.heading(t!("themes:title"));
        if manager.busy {
            ui.spinner();
        }
        ui.add_enabled_ui(!manager.busy, |ui| {
            button(ui, actions, &t!("themes:create"), "create");
            button(ui, actions, &t!("themes:duplicate"), "duplicate");
            button(ui, actions, &t!("themes:import"), "import");
            button(ui, actions, &t!("themes:export"), "export");
            if let Some(draft) = &manager.draft {
                if draft.builtin {
                    button(ui, actions, &t!("themes:resetDefault"), "reset");
                } else {
                    button(ui, actions, &t!("themes:delete"), "delete");
                }
                ui.add_enabled_ui(draft.dirty(), |ui| {
                    button(ui, actions, &t!("themes:save"), "save")
                });
                ui.add_enabled_ui(!draft.dirty(), |ui| {
                    button(ui, actions, &t!("themes:use"), "use")
                });
            }
        });
    });
    ui.separator();
    if let Some(error) = &manager.error {
        ui.colored_label(kit.color(Tok::Danger), error);
        if manager.conflict {
            ui.horizontal(|ui| {
                if ui.button(t!("themes:reload")).clicked()
                    && let Some(draft) = &manager.draft
                {
                    command(
                        actions,
                        json!({"action":"select", "id":draft.id, "discard":true}),
                    );
                }
                if ui.button(t!("themes:overwrite")).clicked() {
                    command(actions, json!({"action":"save", "overwrite":true}));
                }
            });
        }
    }
    let narrow = ui.available_width() < 780.0;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(170.0, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut manager.search)
                        .hint_text(t!("themes:search"))
                        .desired_width(160.0),
                );
                egui::ScrollArea::vertical()
                    .id_salt("theme-list")
                    .show(ui, |ui| {
                        let mut themes: Vec<_> = ThemeId::ALL
                            .into_iter()
                            .map(|id| core.load_theme(id.id()))
                            .collect();
                        themes.extend(
                            core.custom_themes
                                .iter()
                                .filter(|t| ThemeId::from_id(&t.id).is_none())
                                .cloned(),
                        );
                        for theme in themes {
                            if !theme
                                .name
                                .to_lowercase()
                                .contains(&manager.search.to_lowercase())
                            {
                                continue;
                            }
                            let selected = manager.draft.as_ref().is_some_and(|d| d.id == theme.id);
                            if ui.selectable_label(selected, &theme.name).clicked() {
                                command(actions, json!({"action":"select", "id":theme.id}));
                            }
                        }
                    });
            },
        );
        ui.separator();
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            Vec2::new(width, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                if narrow {
                    egui::ScrollArea::vertical()
                        .id_salt("theme-controls")
                        .max_height((ui.available_height() * 0.6).max(140.0))
                        .show(ui, |ui| controls(ui, core, &mut manager, actions));
                    preview(ui, kit, core, &mut manager);
                } else {
                    ui.columns(2, |columns| {
                        egui::ScrollArea::vertical()
                            .id_salt("theme-controls")
                            .show(&mut columns[0], |ui| {
                                controls(ui, core, &mut manager, actions)
                            });
                        preview(&mut columns[1], kit, core, &mut manager);
                    });
                }
            },
        );
    });
    if manager.pending.is_some() {
        egui::Modal::new(egui::Id::new("theme-unsaved")).show(ui.ctx(), |ui| {
            ui.heading(t!("themes:unsaved"));
            ui.label(t!("themes:unsavedDetail"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!manager.busy, egui::Button::new(t!("themes:save")))
                    .clicked()
                {
                    command(actions, json!({"action":"save"}));
                }
                if ui.button(t!("themes:discard")).clicked()
                    && let Some(mut pending) = manager.pending.take()
                {
                    pending["discard"] = json!(true);
                    command(actions, pending);
                }
                if ui
                    .add_enabled(!manager.busy, egui::Button::new(t!("themes:cancel")))
                    .clicked()
                {
                    command(actions, json!({"action":"cancelPending"}));
                }
            });
        });
    }
    if let Some(mut confirmation) = manager.confirmation.clone() {
        egui::Modal::new(egui::Id::new("theme-confirm")).show(ui.ctx(), |ui| {
            let reset = confirmation["action"] == "reset";
            ui.heading(if reset {
                t!("themes:resetDefault")
            } else {
                t!("themes:delete")
            });
            ui.label(if reset {
                t!("themes:resetDetail")
            } else {
                t!("themes:deleteDetail")
            });
            ui.horizontal(|ui| {
                if ui.button(t!("themes:confirm")).clicked() {
                    confirmation["confirmed"] = json!(true);
                    command(actions, confirmation.clone());
                    manager.confirmation = None;
                }
                if ui.button(t!("themes:cancel")).clicked() {
                    manager.confirmation = None;
                }
            });
        });
    }
    core.theme_manager = manager;
}

fn controls(ui: &mut egui::Ui, core: &AppCore, manager: &mut Manager, actions: &mut Actions) {
    let Some(draft) = &manager.draft else {
        return;
    };
    let theme = match draft.theme() {
        Ok(t) => t,
        Err(e) => {
            ui.label(e.to_string());
            return;
        }
    };
    let mut name = theme.name.clone();
    ui.label(t!("themes:name"));
    if ui
        .add_enabled(!draft.builtin, egui::TextEdit::singleline(&mut name))
        .changed()
    {
        edit(actions, vec!["meta".into(), "name".into()], json!(name));
    }
    if !draft.builtin {
        let mut dark = theme.is_dark();
        ui.horizontal(|ui| {
            ui.label(t!("themes:kind"));
            let changed = ui
                .selectable_value(&mut dark, true, t!("themes:dark"))
                .changed()
                | ui.selectable_value(&mut dark, false, t!("themes:light"))
                    .changed();
            if changed {
                edit(
                    actions,
                    vec!["meta".into(), "kind".into()],
                    json!(if dark { "dark" } else { "light" }),
                );
            }
        });
    }
    if draft.dirty() {
        ui.label(RichText::new(t!("themes:modified")).small());
    }
    if draft.builtin {
        ui.label(RichText::new(t!("themes:builtinHint")).small());
    }
    ui.horizontal(|ui| {
        for (page, key) in [
            "themes:colors",
            "themes:typography",
            "themes:display",
            "themes:rules",
        ]
        .iter()
        .enumerate()
        {
            ui.selectable_value(&mut manager.page, page, t!(key));
        }
    });
    ui.separator();
    if manager.page == 3 {
        ui.label(t!("themes:rulesHint"));
        if ui
            .selectable_label(manager.rule.is_none(), t!("themes:global"))
            .clicked()
        {
            manager.rule = None;
        }
        for (kind, rules) in [
            ("languages", &theme.appearance.languages),
            ("extensions", &theme.appearance.extensions),
        ] {
            for key in rules.keys() {
                let selected = manager
                    .rule
                    .as_ref()
                    .is_some_and(|(k, v)| k == kind && v == key);
                let label = if kind == "extensions" {
                    format!(".{key}")
                } else {
                    key.clone()
                };
                if ui.selectable_label(selected, label).clicked() {
                    manager.rule = Some((kind.into(), key.clone()));
                }
            }
        }
        ui.checkbox(&mut manager.rule_language, t!("themes:languageRule"));
        ui.add(
            egui::TextEdit::singleline(&mut manager.rule_input)
                .hint_text(t!("themes:rulePlaceholder")),
        );
        if ui.button(t!("themes:addRule")).clicked() {
            command(
                actions,
                json!({"action":"addRule","key":manager.rule_input,"language":manager.rule_language}),
            );
        }
        if manager.rule.is_some() {
            button(ui, actions, &t!("themes:removeRule"), "removeRule");
        }
        ui.separator();
    }
    if let Some((kind, key)) = &manager.rule {
        ui.label(t!(
            "themes:editingRule",
            rule = if kind == "extensions" {
                format!(".{key}")
            } else {
                key.clone()
            }
        ));
    }
    ui.add(egui::TextEdit::singleline(&mut manager.filter).hint_text(t!("themes:filter")));
    if manager.page == 0 || manager.page == 3 {
        colors(ui, manager, &theme, actions);
    }
    if manager.page != 0 {
        settings(ui, core, manager, &theme, actions);
    }
}

fn colors(ui: &mut egui::Ui, manager: &mut Manager, theme: &Theme, actions: &mut Actions) {
    let rule_theme = if let Some((kind, key)) = &manager.rule {
        if kind == "languages" {
            theme.for_document(Some(key), None)
        } else {
            theme.for_document(None, Some(&format!("preview.{key}")))
        }
    } else {
        theme.clone()
    };
    let doc = manager
        .draft
        .as_ref()
        .and_then(|d| toml::from_str::<toml::Value>(&d.source).ok());
    for group in ["ui", "editor", "syntax", "git", "diff"] {
        if manager.rule.is_some() && !matches!(group, "editor" | "syntax") {
            continue;
        }
        egui::CollapsingHeader::new(t!(&format!("themes:group.{group}")))
            .default_open(group == "ui" || group == "editor")
            .show(ui, |ui| {
                for token in Tok::ALL {
                    let (section, key) = theme_manager::token_path(*token);
                    if section != group
                        || !token
                            .name()
                            .to_lowercase()
                            .contains(&manager.filter.to_lowercase())
                    {
                        continue;
                    }
                    let mut path = if manager.rule.is_some() {
                        root(manager, "colors")
                    } else {
                        vec![section.into()]
                    };
                    path.push(if manager.rule.is_some() {
                        token.name().into()
                    } else {
                        key.into()
                    });
                    let explicit = doc
                        .as_ref()
                        .is_some_and(|d| path.iter().try_fold(d, |v, k| v.get(k)).is_some());
                    ui.push_id(token.name(), |ui| {
                        ui.label(token.name());
                        if !explicit {
                            ui.label(RichText::new(t!("themes:inherited")).small());
                        }
                        let style = rule_theme.style(*token);
                        let mut color = style.color;
                        let input_id = format!("{:?}:{}", manager.rule, token.name());
                        let mut value = manager
                            .color_inputs
                            .get(&input_id)
                            .cloned()
                            .unwrap_or_else(|| theme_manager::hex(style));
                        let mut bold = style.bold;
                        let mut italic = style.italic;
                        ui.horizontal_wrapped(|ui| {
                            let changed = ui.color_edit_button_srgba(&mut color).changed();
                            let input =
                                ui.add(egui::TextEdit::singleline(&mut value).desired_width(90.0));
                            let mut style_changed = false;
                            if group == "syntax" {
                                style_changed |=
                                    ui.checkbox(&mut bold, t!("themes:bold")).changed();
                                style_changed |=
                                    ui.checkbox(&mut italic, t!("themes:italic")).changed();
                            }
                            if changed {
                                value = theme_manager::hex(crate::theme::Style { color, ..style });
                            }
                            if input.changed() {
                                manager.color_inputs.insert(input_id.clone(), value.clone());
                            }
                            if changed || input.changed() || style_changed {
                                edit(
                                    actions,
                                    path.clone(),
                                    json!({"color":value,"bold":bold,"italic":italic}),
                                );
                            }
                            if changed {
                                manager.color_inputs.remove(&input_id);
                            }
                            if ui
                                .add_enabled(explicit, egui::Button::new(t!("themes:reset")))
                                .clicked()
                            {
                                manager.color_inputs.remove(&input_id);
                                edit(actions, path, Value::Null);
                            }
                        });
                    });
                }
            });
    }
}

fn settings(
    ui: &mut egui::Ui,
    core: &AppCore,
    manager: &Manager,
    theme: &Theme,
    actions: &mut Actions,
) {
    let language = manager
        .rule
        .as_ref()
        .filter(|(k, _)| k == "languages")
        .map(|(_, v)| v.as_str());
    let filename = manager
        .rule
        .as_ref()
        .filter(|(k, _)| k == "extensions")
        .map(|(_, v)| format!("preview.{v}"));
    let layer = theme
        .appearance
        .settings_layer(language, filename.as_deref());
    let resolved = core.settings.clone().with_theme_defaults(layer.clone());
    for key in theme_manager::SETTINGS {
        let typography = key.contains("font") || key.contains("lineHeight");
        if manager.page == 1 && !typography
            || manager.page == 2 && typography
            || manager.rule.is_some() && !key.starts_with("editor.")
        {
            continue;
        }
        if !key.to_lowercase().contains(&manager.filter.to_lowercase()) {
            continue;
        }
        let Some(def) = core.schema.get(key) else {
            continue;
        };
        let value = layer.values.get(*key).unwrap_or(&def.default);
        let mut path = root(manager, "settings");
        path.push((*key).into());
        ui.push_id(*key, |ui| {
            ui.label(super::settings_ui::setting_title(key));
            ui.horizontal_wrapped(|ui| {
                match (&def.ty, value) {
                    (SettingType::Bool, SettingValue::Bool(v)) => {
                        let mut v = *v;
                        if ui.checkbox(&mut v, "").changed() {
                            edit(actions, path.clone(), json!(v));
                        }
                    }
                    (SettingType::Int { min, max }, SettingValue::Int(v)) => {
                        let mut v = *v;
                        if ui
                            .add(egui::DragValue::new(&mut v).range(*min..=*max))
                            .changed()
                        {
                            edit(actions, path.clone(), json!(v));
                        }
                    }
                    (SettingType::Float { min, max }, _) => {
                        let mut v = value.as_float().unwrap_or(1.0);
                        if ui
                            .add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.05))
                            .changed()
                        {
                            edit(actions, path.clone(), json!(v));
                        }
                    }
                    (SettingType::Enum(values), SettingValue::Str(v)) => {
                        egui::ComboBox::from_id_salt("value")
                            .selected_text(v)
                            .show_ui(ui, |ui| {
                                for option in *values {
                                    if ui.selectable_label(v == option, *option).clicked() {
                                        edit(actions, path.clone(), json!(option));
                                    }
                                }
                            });
                    }
                    (SettingType::Str, SettingValue::Str(v)) if key.contains("fontFamily") => {
                        let mut text = v.clone();
                        if ui
                            .add(egui::TextEdit::singleline(&mut text).desired_width(145.0))
                            .changed()
                        {
                            edit(actions, path.clone(), json!(text));
                        }
                        egui::ComboBox::from_id_salt("fonts")
                            .selected_text(t!("themes:chooseFont"))
                            .show_ui(ui, |ui| {
                                for font in &manager.fonts {
                                    if ui.selectable_label(font == v, font).clicked() {
                                        edit(actions, path.clone(), json!(font));
                                    }
                                }
                            });
                    }
                    (SettingType::StrList, SettingValue::List(v)) => {
                        let mut text = v.join(", ");
                        if ui
                            .add(egui::TextEdit::singleline(&mut text).desired_width(130.0))
                            .changed()
                        {
                            edit(
                                actions,
                                path.clone(),
                                json!(
                                    text.split(',')
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .collect::<Vec<_>>()
                                ),
                            );
                        }
                    }
                    _ => {}
                }
                if ui.button(t!("themes:reset")).clicked() {
                    edit(actions, path, Value::Null);
                }
            });
            if let Some(scope) = resolved.explicit_source(key, language) {
                let key = match scope {
                    notoir_core::settings::SettingScope::User => "themes:overriddenUser",
                    notoir_core::settings::SettingScope::Workspace => "themes:overriddenWorkspace",
                };
                ui.label(RichText::new(t!(key)).small());
            }
        });
    }
}

fn preview(ui: &mut egui::Ui, kit: &mut Kit, core: &AppCore, manager: &mut Manager) {
    ui.heading(t!("themes:preview"));
    ui.horizontal(|ui| {
        ui.label(t!("themes:extension"));
        if manager.preview_extension.is_empty() {
            manager.preview_extension = "jsx".into();
        }
        ui.add(egui::TextEdit::singleline(&mut manager.preview_extension).desired_width(100.0));
    });
    let Some(theme) = manager.draft.as_ref().and_then(|d| d.theme().ok()) else {
        return;
    };
    let filename = format!(
        "preview.{}",
        manager.preview_extension.trim_start_matches('.')
    );
    let language = notoir_editor::language::detect(Some(&filename), "", &[]);
    let theme = theme.for_document(Some(language.as_str()), Some(&filename));
    let layer = theme
        .appearance
        .settings_layer(Some(language.as_str()), Some(&filename));
    let settings = core.settings.clone().with_theme_defaults(layer);
    // This palette is already resolved for the selected preview extension.
    // Do not reapply language colors when the virtual editor renders it.
    let mut theme = theme;
    theme.appearance.languages.clear();
    theme.appearance.extensions.clear();
    let family = settings.str("editor.fontFamily", Some(language.as_str()));
    crate::fonts::request_family(ui.ctx(), family);
    if !matches!(family, "JetBrains Mono" | "Manrope")
        && !manager.fonts.is_empty()
        && !manager.fonts.iter().any(|f| f == family)
    {
        ui.label(t!("themes:missingFont", font = family));
    }
    let contrast = crate::theme::contrast(
        theme.color(Tok::EditorForeground),
        theme.color(Tok::EditorBackground),
    );
    if contrast < 4.5 {
        ui.colored_label(kit.color(Tok::Warning), t!("themes:contrast"));
    }
    if manager
        .preview
        .as_ref()
        .is_none_or(|p| p.extension != filename)
    {
        let mut workbench = notoir_core::Workbench::new();
        let mut loaded = notoir_core::document::LoadedText::empty(language);
        loaded.read_only = notoir_core::document::ReadOnly::Snapshot;
        loaded.text = if matches!(language.as_str(), "yaml") {
            "# Theme preview\nname: Notoir\nenabled: true\ncolors:\n  accent: \"#7aa2ff\"\n  size: 14\n".into()
        } else {
            "// Theme preview\nconst title = \"Notoir\";\nexport function Welcome({ name }) {\n  return <section className=\"welcome\">\n    <h1>{name}</h1>\n    <span>{42 + 8}</span>\n  </section>;\n}\n".into()
        };
        let doc = workbench.open_document(
            notoir_core::document::DocumentOrigin::Virtual {
                title: "preview".into(),
            },
            loaded,
        );
        let group = workbench.layout.active_group;
        let view = workbench
            .open_view(doc, group, notoir_core::workbench::TabPosition::AfterActive)
            .expect("preview view");
        manager.preview = Some(Box::new(theme_manager::Preview {
            workbench,
            caches: Default::default(),
            view,
            extension: filename,
        }));
    }
    let preview = manager.preview.as_mut().expect("created preview");
    let mut opts = core.editor_options_from_settings(
        notoir_core::ids::ViewId(u64::MAX),
        &settings,
        Some(language.as_str()),
    );
    opts.id_namespace = Some(ui.id().with("theme-preview"));
    let mut preview_kit = Kit {
        theme: &theme,
        icons: kit.icons,
    };
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, kit.theme.color(Tok::Border)))
        .show(ui, |ui| {
            ui.set_min_height(180.0);
            ui.set_min_width(ui.available_width().max(120.0));
            crate::editor_widget::show(
                ui,
                &mut preview_kit,
                &mut preview.workbench,
                &mut preview.caches,
                preview.view,
                &opts,
                false,
                ui.input(|i| i.time),
            );
        });
}
