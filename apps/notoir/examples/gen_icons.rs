//! Regenerates the bundle icons in `assets/app-icon/` from `notoir.svg`:
//! `cargo run -p notoir --example gen_icons`, then (macOS)
//! `iconutil -c icns assets/app-icon/Notoir.iconset -o assets/app-icon/Notoir.icns`
//! and ImageMagick `magick assets/app-icon/png/{16,24,32,48,64,256}x*.png assets/app-icon/Notoir.ico`.

use std::path::PathBuf;

fn render(svg: &[u8], size: u32, inset: f32) -> image::RgbaImage {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).expect("svg");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("pixmap");
    let body = size as f32 * (1.0 - 2.0 * inset);
    let scale = body / tree.size().width();
    let offset = size as f32 * inset;
    let xf = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(offset, offset);
    resvg::render(&tree, xf, &mut pixmap.as_mut());
    // tiny-skia is premultiplied; image wants straight alpha.
    let data: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    image::RgbaImage::from_raw(size, size, data).expect("image")
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/app-icon");
    let svg = std::fs::read(root.join("notoir.svg")).expect("notoir.svg");
    // Full-bleed PNGs (Windows / Linux).
    let png = root.join("png");
    std::fs::create_dir_all(&png).unwrap();
    for size in [16, 24, 32, 48, 64, 128, 256, 512, 1024] {
        render(&svg, size, 0.0)
            .save(png.join(format!("{size}x{size}.png")))
            .unwrap();
    }
    // macOS iconset: Apple's grid keeps the body at ~80 % of the canvas.
    let iconset = root.join("Notoir.iconset");
    std::fs::create_dir_all(&iconset).unwrap();
    for base in [16u32, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = base * scale;
            render(&svg, size, 0.1)
                .save(iconset.join(format!("icon_{base}x{base}{suffix}.png")))
                .unwrap();
        }
    }
    println!("icons written to {}", root.display());
}
