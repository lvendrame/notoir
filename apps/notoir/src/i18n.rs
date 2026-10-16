//! Translations (docs/specs/22-internationalization.md): i18next-style JSON
//! resources embedded at build time. Keys are `"ns:dotted.path"`; `{{var}}`
//! placeholders are interpolated; plural forms use CLDR category suffixes
//! (`_one`, `_few`, `_many`, `_other`). Missing keys fall back to English,
//! then to the key itself.
//!
//! The active language is an application preference owned by the UI thread;
//! it is stored globally so `t!` works anywhere in UI code.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    En,
    PtBr,
    Es,
    Fr,
    De,
    It,
    Nl,
    Pl,
    Ja,
    Ru,
    Uk,
    Tr,
    ZhCn,
    Ko,
}

/// CLDR plural category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    One,
    Few,
    Many,
    Other,
}

impl Plural {
    fn suffix(self) -> &'static str {
        match self {
            Plural::One => "one",
            Plural::Few => "few",
            Plural::Many => "many",
            Plural::Other => "other",
        }
    }
}

impl Language {
    pub const ALL: [Language; 14] = [
        Language::En,
        Language::PtBr,
        Language::Es,
        Language::Fr,
        Language::De,
        Language::It,
        Language::Nl,
        Language::Pl,
        Language::Ja,
        Language::Ru,
        Language::Uk,
        Language::Tr,
        Language::ZhCn,
        Language::Ko,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::PtBr => "pt-BR",
            Language::Es => "es",
            Language::Fr => "fr",
            Language::De => "de",
            Language::It => "it",
            Language::Nl => "nl",
            Language::Pl => "pl",
            Language::Ja => "ja",
            Language::Ru => "ru",
            Language::Uk => "uk",
            Language::Tr => "tr",
            Language::ZhCn => "zh-CN",
            Language::Ko => "ko",
        }
    }

    pub fn native_label(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::PtBr => "Português (BR)",
            Language::Es => "Español",
            Language::Fr => "Français",
            Language::De => "Deutsch",
            Language::It => "Italiano",
            Language::Nl => "Nederlands",
            Language::Pl => "Polski",
            Language::Ja => "日本語",
            Language::Ru => "Русский",
            Language::Uk => "Українська",
            Language::Tr => "Türkçe",
            Language::ZhCn => "简体中文",
            Language::Ko => "한국어",
        }
    }

    /// English name, shown next to the native one in pickers.
    pub fn english_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::PtBr => "Portuguese (Brazil)",
            Language::Es => "Spanish",
            Language::Fr => "French",
            Language::De => "German",
            Language::It => "Italian",
            Language::Nl => "Dutch",
            Language::Pl => "Polish",
            Language::Ja => "Japanese",
            Language::Ru => "Russian",
            Language::Uk => "Ukrainian",
            Language::Tr => "Turkish",
            Language::ZhCn => "Chinese (Simplified)",
            Language::Ko => "Korean",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        Language::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(code))
    }

    /// Needs a CJK-capable UI font.
    pub fn is_cjk(self) -> bool {
        matches!(self, Language::Ja | Language::ZhCn | Language::Ko)
    }

    /// A stored choice wins; otherwise the OS locale prefix decides (spec 22).
    pub fn detect(os_locale: Option<&str>, stored: Option<&str>) -> Language {
        if let Some(l) = stored.and_then(Language::from_code) {
            return l;
        }
        let lower = os_locale.unwrap_or("en").to_ascii_lowercase();
        let table = [
            ("pt", Language::PtBr),
            ("es", Language::Es),
            ("fr", Language::Fr),
            ("de", Language::De),
            ("it", Language::It),
            ("nl", Language::Nl),
            ("pl", Language::Pl),
            ("ja", Language::Ja),
            ("ru", Language::Ru),
            ("uk", Language::Uk),
            ("tr", Language::Tr),
            ("zh", Language::ZhCn),
            ("ko", Language::Ko),
        ];
        table
            .into_iter()
            .find(|(p, _)| lower.starts_with(p))
            .map(|(_, l)| l)
            .unwrap_or(Language::En)
    }

    /// CLDR cardinal plural category for an integer count.
    pub fn plural(self, n: u64) -> Plural {
        let (m10, m100) = (n % 10, n % 100);
        match self {
            Language::Ja | Language::ZhCn | Language::Ko => Plural::Other,
            Language::PtBr | Language::Fr => {
                if n <= 1 {
                    Plural::One
                } else {
                    Plural::Other
                }
            }
            Language::Pl => {
                if n == 1 {
                    Plural::One
                } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                    Plural::Few
                } else {
                    Plural::Many
                }
            }
            Language::Ru | Language::Uk => {
                if m10 == 1 && m100 != 11 {
                    Plural::One
                } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                    Plural::Few
                } else {
                    Plural::Many
                }
            }
            Language::En
            | Language::Es
            | Language::De
            | Language::It
            | Language::Nl
            | Language::Tr => {
                if n == 1 {
                    Plural::One
                } else {
                    Plural::Other
                }
            }
        }
    }
}

type Table = HashMap<String, String>;

fn flatten(prefix: &str, v: &Value, out: &mut Table) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                let key = if prefix.ends_with(':') {
                    format!("{prefix}{k}")
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, child, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
        }
        other => {
            out.insert(prefix.to_owned(), other.to_string());
        }
    }
}

fn tables() -> &'static HashMap<&'static str, Table> {
    static TABLES: OnceLock<HashMap<&'static str, Table>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut all: HashMap<&'static str, Table> = HashMap::new();
        for (lang, ns, json) in LOCALE_FILES {
            let value: Value = serde_json::from_str(json).unwrap_or(Value::Null);
            flatten(&format!("{ns}:"), &value, all.entry(lang).or_default());
        }
        all
    })
}

/// Every flattened key for `lang`, sorted (parity checks).
pub fn keys(lang: Language) -> Vec<String> {
    let mut k: Vec<String> = tables()
        .get(lang.code())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    k.sort();
    k
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn set_language(lang: Language) {
    let index = Language::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    CURRENT.store(index, Ordering::Relaxed);
}

pub fn language() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed) % Language::ALL.len()]
}

fn lookup(lang: Language, key: &str) -> Option<&'static String> {
    let t = tables();
    t.get(lang.code())
        .and_then(|m| m.get(key))
        .or_else(|| t.get("en").and_then(|m| m.get(key)))
}

/// Strings registered at run time (plugin command titles), keyed
/// `@plugin/<id>/<key>`; they already are in the current language.
fn dynamic() -> &'static std::sync::RwLock<HashMap<String, String>> {
    static DYNAMIC: OnceLock<std::sync::RwLock<HashMap<String, String>>> = OnceLock::new();
    DYNAMIC.get_or_init(Default::default)
}

pub fn set_dynamic(key: &str, value: &str) {
    if let Ok(mut d) = dynamic().write() {
        d.insert(key.to_owned(), value.to_owned());
    }
}

/// Drops every dynamic string starting with `prefix`.
pub fn clear_dynamic(prefix: &str) {
    if let Ok(mut d) = dynamic().write() {
        d.retain(|k, _| !k.starts_with(prefix));
    }
}

/// Raw lookup in `lang`; returns the key itself when missing everywhere.
pub fn t_in(lang: Language, key: &str) -> String {
    if key.starts_with('@')
        && let Some(s) = dynamic().read().ok().and_then(|d| d.get(key).cloned())
    {
        return s;
    }
    match lookup(lang, key) {
        Some(s) => s.clone(),
        None => {
            // Run-time strings (plugin setting titles under `settings:`).
            if let Some(s) = dynamic().read().ok().and_then(|d| d.get(key).cloned()) {
                return s;
            }
            tracing::debug!(key, "missing translation key");
            key.to_owned()
        }
    }
}

fn interpolate(mut s: String, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    for (name, value) in args {
        s = s.replace(&format!("{{{{{name}}}}}"), &value.to_string());
    }
    s
}

/// Lookup in `lang` with `{{name}}` interpolation.
pub fn tf_in(lang: Language, key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    interpolate(t_in(lang, key), args)
}

/// Plural lookup in `lang`: `key_<category>`, then `key_other`; `{{count}}` is
/// always available for interpolation.
pub fn tn_in(
    lang: Language,
    key: &str,
    count: u64,
    args: &[(&str, &dyn std::fmt::Display)],
) -> String {
    let category = lang.plural(count).suffix();
    let template = lookup(lang, &format!("{key}_{category}"))
        .or_else(|| lookup(lang, &format!("{key}_other")))
        .cloned()
        .unwrap_or_else(|| key.to_owned());
    let mut all: Vec<(&str, &dyn std::fmt::Display)> = vec![("count", &count)];
    all.extend_from_slice(args);
    interpolate(template, &all)
}

/// `t!("ns:key")` or `t!("ns:key", name = value, ...)` in the active language.
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::t_in($crate::i18n::language(), $key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tf_in(
            $crate::i18n::language(),
            $key,
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

/// `tn!("ns:key", count = n)` (plus optional named args) in the active language.
#[macro_export]
macro_rules! tn {
    ($key:expr, count = $count:expr $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::tn_in(
            $crate::i18n::language(),
            $key,
            $count as u64,
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),*],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys with the plural suffix removed: plural forms legitimately differ
    /// between languages, so parity is checked on the base key.
    fn base_keys(lang: Language) -> Vec<String> {
        let mut k: Vec<String> = keys(lang)
            .into_iter()
            .map(|k| {
                for s in ["_zero", "_one", "_two", "_few", "_many", "_other"] {
                    if let Some(base) = k.strip_suffix(s) {
                        return base.to_owned();
                    }
                }
                k
            })
            .collect();
        k.sort();
        k.dedup();
        k
    }

    // spec 22: "All locales have the same keys"
    #[test]
    fn all_locales_have_identical_keys() {
        let en = base_keys(Language::En);
        assert!(!en.is_empty());
        for lang in Language::ALL {
            assert_eq!(base_keys(lang), en, "key parity for {}", lang.code());
        }
    }

    #[test]
    fn every_plural_key_has_each_category_its_language_uses() {
        for lang in Language::ALL {
            let all = keys(lang);
            let plurals: Vec<&str> = all
                .iter()
                .filter_map(|k| k.strip_suffix("_other"))
                .collect();
            for base in plurals {
                for n in [0, 1, 2, 3, 5, 11, 12, 21, 22, 25, 101] {
                    let key = format!("{base}_{}", lang.plural(n).suffix());
                    assert!(all.contains(&key), "{} misses {key}", lang.code());
                }
            }
        }
    }

    #[test]
    fn english_lookup_interpolation_and_fallback() {
        assert_eq!(t_in(Language::En, "welcome:startWriting"), "Start writing.");
        assert_eq!(t_in(Language::En, "missing:key"), "missing:key");
        assert_eq!(tn_in(Language::En, "common:fileCount", 1, &[]), "1 file");
        assert_eq!(tn_in(Language::En, "common:fileCount", 4, &[]), "4 files");
    }

    // spec 22: "Unsupported locale falls back to English"
    #[test]
    fn detect() {
        assert_eq!(Language::detect(Some("pt-PT"), None), Language::PtBr);
        assert_eq!(Language::detect(Some("zh-TW"), None), Language::ZhCn);
        assert_eq!(Language::detect(Some("ja-JP"), None), Language::Ja);
        assert_eq!(Language::detect(Some("sv-SE"), None), Language::En);
        assert_eq!(Language::detect(None, None), Language::En);
        assert_eq!(Language::detect(Some("fr"), Some("ja")), Language::Ja);
    }

    // spec 22: "Polish plural forms"
    #[test]
    fn polish_plural_forms() {
        let f = |n| tn_in(Language::Pl, "common:fileCount", n, &[]);
        assert_eq!(f(1), "1 plik");
        assert_eq!(f(3), "3 pliki");
        assert_eq!(f(5), "5 plików");
        assert_eq!(f(22), "22 pliki");
        assert_eq!(f(12), "12 plików");
    }

    #[test]
    fn russian_and_japanese_plural_categories() {
        assert_eq!(Language::Ru.plural(21), Plural::One);
        assert_eq!(Language::Ru.plural(11), Plural::Many);
        assert_eq!(Language::Ru.plural(24), Plural::Few);
        assert_eq!(Language::Ja.plural(1), Plural::Other);
    }

    #[test]
    fn every_language_is_translated() {
        let en = t_in(Language::En, "welcome:startWriting");
        for lang in Language::ALL.into_iter().skip(1) {
            assert_ne!(t_in(lang, "welcome:startWriting"), en, "{}", lang.code());
        }
    }

    #[test]
    fn macros_use_active_language() {
        set_language(Language::De);
        assert_eq!(crate::t!("welcome:recent"), "Zuletzt verwendet");
        assert_eq!(crate::tn!("common:fileCount", count = 2), "2 Dateien");
        set_language(Language::En);
    }
}
