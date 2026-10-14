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

pub fn install(ctx: &egui::Context, lang: Language) {
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
    if let Some(f) = script_for(lang).and_then(system_fonts::cjk_font) {
        fallbacks.push(fallback("cjk-system", f.bytes, f.index));
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
    ctx.set_fonts(fonts);
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
