//! UI and editor fonts (docs/specs/21-themes-and-design-system.md
//! §Typography): Manrope for chrome text, JetBrains Mono for the editor and
//! numeric readouts. Both are bundled (OFL). CJK UI languages add an installed
//! system face as fallback (spec 22); a tiny Noto Sans CJK subset always covers
//! the native language names in language pickers.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};
use notoir_platform::system_fonts::{self, Script};

use crate::i18n::Language;

pub const MANROPE_SEMIBOLD: &str = "manrope-semibold";
pub const MANROPE_EXTRABOLD: &str = "manrope-extrabold";
pub const MONO_SEMIBOLD: &str = "jetbrains-mono-semibold";

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/ui/",
            $file
        ))
    };
}

/// Script whose system font the UI needs for `lang`.
pub fn script_for(lang: Language) -> Option<Script> {
    match lang {
        Language::Ja => Some(Script::Japanese),
        Language::ZhCn => Some(Script::SimplifiedChinese),
        Language::Ko => Some(Script::Korean),
        _ => None,
    }
}

/// Installs the font families. `cjk` are the system faces found so far for
/// the scripts in use (UI language and editor text).
pub fn install(ctx: &egui::Context, cjk: &[system_fonts::SystemFont]) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("manrope", font!("Manrope-Regular.ttf"));
    add(MANROPE_SEMIBOLD, font!("Manrope-SemiBold.ttf"));
    add(MANROPE_EXTRABOLD, font!("Manrope-ExtraBold.ttf"));
    add("jetbrains-mono", font!("JetBrainsMono-Regular.ttf"));
    add(MONO_SEMIBOLD, font!("JetBrainsMono-SemiBold.ttf"));

    // CJK fallbacks, shifted onto Manrope's baseline.
    let manrope: &[u8] = font!("Manrope-Regular.ttf");
    let mut fallback = |name: &str, bytes: &'static [u8], index: u32| {
        let mut data = FontData::from_static(bytes);
        data.index = index;
        data.tweak.y_offset_factor = system_fonts::baseline_offset_factor(manrope, bytes, index);
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
        name.to_owned()
    };
    let mut fallbacks = Vec::new();
    for f in cjk {
        fallbacks.push(fallback(
            &format!("cjk-system-{:?}", f.script),
            f.bytes,
            f.index,
        ));
    }
    fallbacks.push(fallback(
        "cjk-language-names",
        font!("NotoSansCJK-LanguageNames.otf"),
        0,
    ));

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "manrope".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains-mono".to_owned());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .extend(fallbacks.iter().cloned());
    }
    for (weight, base) in [
        (MANROPE_SEMIBOLD, FontFamily::Proportional),
        (MANROPE_EXTRABOLD, FontFamily::Proportional),
        (MONO_SEMIBOLD, FontFamily::Monospace),
    ] {
        let mut chain = vec![weight.to_owned()];
        chain.extend(fonts.families[&base].iter().skip(1).cloned());
        fonts
            .families
            .insert(FontFamily::Name(weight.into()), chain);
    }
    for face in system_fonts::family_faces() {
        let key = family_key(&face.family, face.bold, face.italic);
        let mut data = FontData::from_owned(face.bytes.to_vec());
        data.index = face.index;
        fonts.font_data.insert(key.clone(), Arc::new(data));
        let mut chain = vec![key.clone()];
        chain.extend(fonts.families[&FontFamily::Monospace].iter().cloned());
        fonts.families.insert(FontFamily::Name(key.into()), chain);
    }
    ctx.set_fonts(fonts);
    ctx.data_mut(|data| {
        let generation = data.get_temp_mut_or_default::<u64>(egui::Id::new("notoir-font-revision"));
        *generation += 1;
    });
}

/// CJK scripts whose glyphs appear in `text`. Han ideographs without kana or
/// hangul use the UI language's preference (Japanese UI → Japanese face).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CjkUse {
    pub han: bool,
    pub kana: bool,
    pub hangul: bool,
}

impl CjkUse {
    pub fn scan(&mut self, text: &str) {
        if text.is_ascii() {
            return;
        }
        for c in text.chars() {
            match c as u32 {
                0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => self.kana = true,
                0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => self.hangul = true,
                0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => self.han = true,
                _ => {}
            }
        }
    }

    pub fn any(&self) -> bool {
        self.han || self.kana || self.hangul
    }

    pub fn merge(&mut self, other: CjkUse) {
        self.han |= other.han;
        self.kana |= other.kana;
        self.hangul |= other.hangul;
    }

    /// System font scripts needed, given the UI language.
    pub fn scripts(&self, ui: Language) -> Vec<Script> {
        let mut out = Vec::new();
        if self.kana {
            out.push(Script::Japanese);
        }
        if self.hangul {
            out.push(Script::Korean);
        }
        if self.han && !self.kana {
            out.push(match ui {
                Language::Ja => Script::Japanese,
                Language::Ko => Script::Korean,
                _ => Script::SimplifiedChinese,
            });
        }
        out
    }
}

/// Bold/semibold text uses a named family since egui has no weight axis.
pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(MANROPE_SEMIBOLD.into()))
}

pub fn extrabold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(MANROPE_EXTRABOLD.into()))
}

pub fn mono(size: f32) -> egui::FontId {
    egui::FontId::monospace(size)
}

pub fn family_key(family: &str, bold: bool, italic: bool) -> String {
    format!("user:{family}:{bold}:{italic}")
}

pub fn family(family: &str, bold: bool, italic: bool) -> FontFamily {
    if family == "JetBrains Mono" {
        return if bold {
            FontFamily::Name(MONO_SEMIBOLD.into())
        } else {
            FontFamily::Monospace
        };
    }
    if family == "Manrope" {
        return if bold {
            FontFamily::Name(MANROPE_SEMIBOLD.into())
        } else {
            FontFamily::Proportional
        };
    }
    if system_fonts::family_faces()
        .iter()
        .any(|f| f.family == family)
    {
        FontFamily::Name(family_key(family, bold, italic).into())
    } else {
        FontFamily::Monospace
    }
}

/// The process-wide font cache is a union, preserving families used by siblings.
pub fn request_family(ctx: &egui::Context, family: &str) {
    let repaint = ctx.clone();
    system_fonts::request_family(family, move || repaint.request_repaint());
}

/// Apply typography locally before rendering each window.
pub fn apply_ui(ctx: &egui::Context, settings: &notoir_core::settings::ResolvedSettings) {
    let selected = settings.str("workbench.fontFamily", None);
    request_family(ctx, selected);
    let font = egui::FontId::new(
        settings.int("workbench.fontSize", None) as f32,
        available_family(ctx, selected, false, false),
    );
    ctx.all_styles_mut(|style| {
        style.override_font_id = None;
        for (role, extra) in [
            (egui::TextStyle::Body, 0.0),
            (egui::TextStyle::Button, 0.0),
            (egui::TextStyle::Heading, 5.0),
            (egui::TextStyle::Small, -2.0),
        ] {
            style.text_styles.insert(
                role,
                egui::FontId::new(font.size + extra, font.family.clone()),
            );
        }
    });
    let zoom = settings.float("workbench.uiScale", None) as f32;
    if (ctx.zoom_factor() - zoom).abs() > 0.001 {
        ctx.set_zoom_factor(zoom);
    }
}

pub fn available_family(
    ctx: &egui::Context,
    selected: &str,
    bold: bool,
    italic: bool,
) -> FontFamily {
    let wanted = family(selected, bold, italic);
    if ctx.fonts_mut(|f| f.families().contains(&wanted)) {
        wanted
    } else {
        FontFamily::Monospace
    }
}

pub fn revision(ctx: &egui::Context) -> u64 {
    ctx.data_mut(|data| {
        data.get_temp::<u64>(egui::Id::new("notoir-font-revision"))
            .unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_cover_native_language_names() {
        let manrope: &[u8] = font!("Manrope-Regular.ttf");
        let noto: &[u8] = font!("NotoSansCJK-LanguageNames.otf");
        for lang in Language::ALL {
            for c in lang.native_label().chars() {
                assert!(
                    system_fonts::has_glyph(manrope, 0, c) || system_fonts::has_glyph(noto, 0, c),
                    "{c:?} in {}",
                    lang.native_label()
                );
            }
        }
    }
}
