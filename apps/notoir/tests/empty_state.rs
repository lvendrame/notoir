//! Empty state (spec 17) rendered in the active language (spec 22).

mod common;

use egui_kittest::kittest::Queryable;
use notoir::i18n::Language;
use notoir::theme::ThemeId;

#[test]
fn empty_state_is_translated() {
    // One test so the process-wide active language is not shared across threads.
    for (lang, text) in [
        (Language::En, "Start writing."),
        (Language::PtBr, "Comece a escrever."),
        (Language::Ja, "書き始めましょう。"),
    ] {
        let app = common::start(common::with_language(lang, ThemeId::Midnight));
        assert!(
            app.harness.query_by_label(text).is_some(),
            "{} empty state missing {text:?}",
            lang.code()
        );
    }
}
