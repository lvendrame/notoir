//! Empty state (spec 17) rendered in the active language (spec 22).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use notoir::app::NotoirApp;
use notoir::i18n::Language;
use notoir::theme::ThemeId;

fn harness(lang: Language) -> Harness<'static, NotoirApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| NotoirApp::with_context(&cc.egui_ctx, lang, ThemeId::Midnight));
    h.run_steps(2);
    h
}

#[test]
fn empty_state_is_translated() {
    // One test so the process-wide active language is not shared across threads.
    for (lang, text) in [
        (Language::En, "Start writing."),
        (Language::PtBr, "Comece a escrever."),
        (Language::Ja, "書き始めましょう。"),
    ] {
        let h = harness(lang);
        assert!(
            h.query_by_label(text).is_some(),
            "{} empty state missing {text:?}",
            lang.code()
        );
    }
}
