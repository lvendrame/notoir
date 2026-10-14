//! Renders key screens to PNG (GPU via wgpu) for visual review: every
//! built-in theme × (en, ja, de). Output goes to `$NOTOIR_SNAPSHOT_DIR`
//! (defaults to `target/ui-snapshots`). Skips without a GPU adapter.

use egui_kittest::Harness;
use notoir::app::NotoirApp;
use notoir::i18n::Language;
use notoir::theme::ThemeId;

fn out_dir() -> std::path::PathBuf {
    let dir = std::env::var("NOTOIR_SNAPSHOT_DIR")
        .map(Into::into)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots")
        });
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn empty_state_snapshots() {
    for theme in ThemeId::ALL {
        for lang in [Language::En, Language::Ja, Language::De] {
            let mut h = Harness::builder()
                .with_size(egui::vec2(1280.0, 800.0))
                .with_pixels_per_point(1.0)
                .build_eframe(move |cc| NotoirApp::with_context(&cc.egui_ctx, lang, theme));
            h.run_steps(4);
            let name = format!("empty-state-{}-{}.png", theme.id(), lang.code());
            match h.render() {
                Ok(img) => img.save(out_dir().join(name)).unwrap(),
                Err(e) => {
                    eprintln!("render skipped (no GPU adapter?): {e}");
                    return;
                }
            }
        }
    }
}
