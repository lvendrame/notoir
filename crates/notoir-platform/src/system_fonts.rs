//! System CJK fallback fonts (docs/specs/22-internationalization.md). The
//! bundled UI fonts (Manrope / JetBrains Mono) have no CJK glyphs, so fontdb
//! locates an installed face for the needed script; it is read once and kept
//! for the process (only CJK users pay for it).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Japanese,
    SimplifiedChinese,
    Korean,
}

impl Script {
    pub const ALL: [Script; 3] = [Script::Japanese, Script::SimplifiedChinese, Script::Korean];

    /// Preferred families, macOS → Windows → Linux.
    fn families(self) -> &'static [&'static str] {
        match self {
            Script::Japanese => &[
                "Hiragino Sans",
                "Hiragino Kaku Gothic ProN",
                "Yu Gothic UI",
                "Yu Gothic",
                "Meiryo",
                "Noto Sans CJK JP",
                "Noto Sans JP",
                "Source Han Sans JP",
                "IPAexGothic",
            ],
            Script::SimplifiedChinese => &[
                "PingFang SC",
                "Hiragino Sans GB",
                "Microsoft YaHei UI",
                "Microsoft YaHei",
                "Noto Sans CJK SC",
                "Noto Sans SC",
                "Source Han Sans SC",
                "WenQuanYi Micro Hei",
                "Heiti SC",
                "STHeiti",
            ],
            Script::Korean => &[
                "Apple SD Gothic Neo",
                "Malgun Gothic",
                "Noto Sans CJK KR",
                "Noto Sans KR",
                "Source Han Sans KR",
                "NanumGothic",
            ],
        }
    }

    /// A character the face must contain.
    fn probe(self) -> char {
        match self {
            Script::Japanese => 'あ',
            Script::SimplifiedChinese => '简',
            Script::Korean => '한',
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SystemFont {
    pub bytes: &'static [u8],
    pub index: u32,
    pub script: Script,
}

fn database() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

fn cache() -> &'static Mutex<HashMap<u8, Option<SystemFont>>> {
    static CACHE: OnceLock<Mutex<HashMap<u8, Option<SystemFont>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The installed face for `script`, if any (scans/reads once per script).
/// Blocks for the system font scan (~1 s cold); UI code uses [`request`].
pub fn cjk_font(script: Script) -> Option<SystemFont> {
    if let Some(found) = ready(script) {
        return found;
    }
    let found = find(database(), script);
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(script as u8, found);
    found
}

/// Non-blocking: `Some(result)` once the lookup for `script` finished.
pub fn ready(script: Script) -> Option<Option<SystemFont>> {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(script as u8))
        .copied()
}

/// Starts the lookup for `script` on a background thread (no-op if done or
/// running) and calls `on_done` when it finishes.
pub fn request(script: Script, on_done: impl FnOnce() + Send + 'static) {
    static RUNNING: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
    if ready(script).is_some() {
        return;
    }
    let mut running = RUNNING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if running.contains(&(script as u8)) {
        return;
    }
    running.push(script as u8);
    let spawned = std::thread::Builder::new()
        .name("notoir-fonts".into())
        .spawn(move || {
            cjk_font(script);
            on_done();
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start font scan thread");
    }
}

fn find(db: &fontdb::Database, script: Script) -> Option<SystemFont> {
    for family in script.families() {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            ..Default::default()
        };
        let Some(id) = db.query(&query) else { continue };
        let Some(face) = db.face(id) else { continue };
        let fontdb::Source::File(path) = &face.source else {
            continue;
        };
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let ok = has_glyph(&data, face.index, script.probe());
        if ok {
            tracing::info!("CJK fallback {script:?}: {family} ({})", path.display());
            // Lives for the process; at most one leak per script.
            let bytes: &'static [u8] = Box::leak(data.into_boxed_slice());
            return Some(SystemFont {
                bytes,
                index: face.index,
                script,
            });
        }
    }
    tracing::warn!("no system font found for {script:?}");
    None
}

/// egui places a fallback glyph at `ascent + ½·(primary_height − face_height)`
/// (heights include the line gap), so faces with different vertical metrics
/// drift off the primary baseline. Returns the downward shift, as a fraction of
/// the font size, that puts `fallback` on `primary`'s baseline.
pub fn baseline_offset_factor(primary: &[u8], fallback: &[u8], fallback_index: u32) -> f32 {
    let m = |data: &[u8], index: u32| {
        let f = FontRef::from_index(data, index).ok()?;
        let metrics = f.metrics(Size::unscaled(), LocationRef::default());
        let upm = f32::from(metrics.units_per_em);
        let (asc, desc, gap) = (
            metrics.ascent / upm,
            metrics.descent / upm,
            metrics.leading / upm,
        );
        Some((asc, asc - desc + gap))
    };
    let (Some((pa, ph)), Some((fa, fh))) = (m(primary, 0), m(fallback, fallback_index)) else {
        return 0.0;
    };
    pa - (fa + 0.5 * (ph - fh))
}

/// Whether the face at `index` in `data` maps `c` to a glyph.
pub fn has_glyph(data: &[u8], index: u32, c: char) -> bool {
    FontRef::from_index(data, index).is_ok_and(|f| f.charmap().map(c).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_fonts_need_no_offset() {
        let manrope = include_bytes!("../../../assets/fonts/ui/Manrope-Regular.ttf");
        assert_eq!(baseline_offset_factor(manrope, manrope, 0), 0.0);
        let noto = include_bytes!("../../../assets/fonts/ui/NotoSansCJK-LanguageNames.otf");
        assert!(baseline_offset_factor(manrope, noto, 0).abs() < 0.2);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn finds_all_three_scripts_on_macos() {
        for s in Script::ALL {
            let f = cjk_font(s).unwrap_or_else(|| panic!("{s:?}"));
            assert_eq!(f.script, s);
        }
    }
}
