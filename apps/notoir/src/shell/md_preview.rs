//! Markdown preview (docs/specs/24 §Preview Rendering): renders the parsed
//! render tree with egui widgets — headings, paragraphs with inline styles,
//! links, images, quotes, lists with clickable task boxes, tables, highlighted
//! code blocks, rules, footnotes and front matter — in a centred column, and
//! records block positions for scroll sync.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_editor::markdown::{Align, Block, BlockKind, Inline, MdDoc};
use notoir_editor::{LanguageId, Rope};
use serde_json::json;

use super::{Actions, Kit};
use crate::t;
use crate::theme::{Theme, Tok};

/// A preview image ready to draw, or why it is not.
pub enum ImageTex {
    Ready(egui::TextureHandle, Vec2),
    Missing(String),
}

/// Per-view preview state (scroll sync bookkeeping).
#[derive(Default)]
pub struct PreviewView {
    /// Scroll offset of the preview content.
    pub offset: f32,
    /// Apply this offset next frame (sync from the editor).
    pub set_offset: Option<f32>,
    /// Top-level blocks: (first line, last line, top, bottom) in content px.
    pub blocks: Vec<(usize, usize, f32, f32)>,
    pub last_editor_top: Option<usize>,
    /// Offset the preview had after our last sync (user scroll = differs).
    pub synced_offset: Option<f32>,
}

#[derive(Default)]
pub struct Previews {
    pub views: HashMap<ViewId, PreviewView>,
    pub images: HashMap<PathBuf, ImageTex>,
}

#[derive(Clone, Copy, Default)]
struct Style {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
}

struct Ctx<'a> {
    doc: DocumentId,
    /// Lowercase wiki link targets with no note (styled as missing).
    missing: std::collections::HashSet<String>,
    base_dir: Option<PathBuf>,
    size: f32,
    interactive: bool,
    remote: &'a str,
}

/// Code block text highlighted with the theme's syntax colours.
pub fn code_job(theme: &Theme, lang: &str, text: &str, font: FontId) -> LayoutJob {
    let plain = theme.color(Tok::EditorForeground);
    let mut job = LayoutJob::default();
    let id = LanguageId::from_id(&lang.to_lowercase())
        .or_else(|| {
            let d = notoir_editor::language::detect(Some(&format!("x.{lang}")), "", &[]);
            (d != LanguageId::PLAINTEXT).then_some(d)
        })
        .unwrap_or(LanguageId::PLAINTEXT);
    // No trailing empty line.
    let rope = Rope::from_str(text.strip_suffix('\n').unwrap_or(text));
    let mut hl = notoir_editor::syntax::Highlighter::new(id, &rope);
    if let Some(h) = hl.as_mut() {
        h.highlight_to(
            &rope,
            rope.len_lines(),
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        );
    }
    for line in 0..rope.len_lines() {
        let text: Vec<char> = rope.line(line).chars().collect();
        let spans = hl
            .as_ref()
            .and_then(|h| h.spans(line))
            .cloned()
            .unwrap_or_default();
        let mut pos = 0;
        let put = |from: usize, to: usize, color: Color32, job: &mut LayoutJob| {
            if to > from {
                let s: String = text[from..to].iter().collect();
                job.append(
                    &s,
                    0.0,
                    TextFormat {
                        font_id: font.clone(),
                        color,
                        ..Default::default()
                    },
                );
            }
        };
        for s in spans.iter() {
            put(pos, s.start.min(text.len()), plain, &mut job);
            let color = crate::theme::syntax_token(s.kind).map_or(plain, |t| theme.color(t));
            put(
                s.start.min(text.len()),
                s.end.min(text.len()),
                color,
                &mut job,
            );
            pos = s.end.min(text.len());
        }
        put(pos, text.len(), plain, &mut job);
    }
    job
}

fn cmd(actions: &mut Actions, id: &'static str, args: serde_json::Value) {
    actions.push(CommandInvocation::new(id, InvocationSource::Internal).with_args(args));
}

/// Renders the preview of `md` (for document `doc`) into `ui`.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    core: &mut crate::core::AppCore,
    view: ViewId,
    doc: DocumentId,
    md: &MdDoc,
    previews: &mut Previews,
    actions: &mut Actions,
) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, kit.color(Tok::BgEditor));
    let rope = core
        .workbench
        .document(doc)
        .map(|d| d.buffer.rope().clone())
        .unwrap_or_default();
    let base_dir = core
        .workbench
        .document(doc)
        .and_then(|d| d.path())
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let size = core.settings.int("editor.fontSize", Some("markdown")) as f32;
    let max_w = core.settings.int("markdown.preview.maxWidth", None) as f32;
    let interactive = core
        .settings
        .bool("markdown.preview.interactiveCheckboxes", None);
    let remote = core
        .settings
        .str("markdown.preview.loadRemoteImages", None)
        .to_owned();
    let state = previews.views.entry(view).or_default();
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("md-preview", view))
        .auto_shrink([false, false]);
    if let Some(o) = state.set_offset.take() {
        scroll = scroll.vertical_scroll_offset(o);
        state.synced_offset = Some(o);
    }
    let mut blocks_out: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut images_wanted: Vec<PathBuf> = Vec::new();
    let missing = notoir_editor::wiki::links(&rope.to_string())
        .into_iter()
        .filter(|l| !l.target.is_empty() && core.notes_missing(doc, &l.target))
        .map(|l| l.target.to_lowercase())
        .collect();
    let cx = Ctx {
        doc,
        missing,
        base_dir,
        size,
        interactive,
        remote: &remote,
    };
    let images = &previews.images;
    let out = scroll.show(ui, |ui| {
        let avail = ui.available_width();
        let w = max_w.min(avail - 32.0).max(120.0);
        let pad = ((avail - w) / 2.0).max(16.0);
        ui.add_space(16.0);
        let top = ui.cursor().top();
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = size * 0.6;
                for b in &md.blocks {
                    let y0 = ui.cursor().top() - top;
                    block(ui, kit, &cx, b, images, &mut images_wanted, actions);
                    let y1 = ui.cursor().top() - top;
                    let l0 = rope.char_to_line(b.range.0.min(rope.len_chars()));
                    let l1 = rope.char_to_line(b.range.1.min(rope.len_chars()));
                    blocks_out.push((l0, l1, y0, y1));
                    // Double-click: caret to this block's source.
                    let r = Rect::from_min_max(
                        Pos2::new(ui.min_rect().left(), y0 + top),
                        Pos2::new(ui.min_rect().right(), y1 + top),
                    );
                    if ui.input(|i| {
                        i.pointer
                            .button_double_clicked(egui::PointerButton::Primary)
                    }) && ui
                        .input(|i| i.pointer.interact_pos())
                        .is_some_and(|p| r.contains(p))
                    {
                        cmd(
                            actions,
                            "markdown.revealSource",
                            json!({ "doc": doc.0, "pos": b.range.0 }),
                        );
                    }
                }
            });
        });
        // Room to bring the last block to the top (scroll sync).
        ui.add_space(rect.height());
    });
    let state = previews.views.entry(view).or_default();
    state.offset = out.state.offset.y;
    state.blocks = blocks_out;
    for p in images_wanted {
        core.request_image(&p);
    }
}

fn block(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    cx: &Ctx,
    b: &Block,
    images: &HashMap<PathBuf, ImageTex>,
    wanted: &mut Vec<PathBuf>,
    actions: &mut Actions,
) {
    let text = kit.color(Tok::TextPrimary);
    match &b.kind {
        BlockKind::Heading { level, inlines, .. } => {
            let scale = [2.0, 1.6, 1.35, 1.15, 1.0, 0.9][(*level as usize).clamp(1, 6) - 1];
            ui.add_space(cx.size * 0.4);
            inline_flow(
                ui,
                kit,
                cx,
                inlines,
                cx.size * scale,
                true,
                images,
                wanted,
                actions,
            );
            if *level <= 2 {
                let r = ui.min_rect();
                let y = ui.cursor().top() - 2.0;
                ui.painter().line_segment(
                    [Pos2::new(r.left(), y), Pos2::new(r.right(), y)],
                    Stroke::new(1.0, kit.color(Tok::Border)),
                );
            }
        }
        BlockKind::Paragraph(inlines) => inline_flow(
            ui, kit, cx, inlines, cx.size, false, images, wanted, actions,
        ),
        BlockKind::Code { lang, text: code } => {
            let job = code_job(kit.theme, lang, code, crate::fonts::mono(cx.size * 0.9));
            egui::Frame::new()
                .fill(kit.color(Tok::BgPanel))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Long lines scroll sideways instead of being cut off.
                    egui::ScrollArea::horizontal()
                        .id_salt(("md-code", b.range.0))
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        )
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
                        });
                });
        }
        BlockKind::Quote(children) => {
            let r = ui
                .horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        for c in children {
                            block(ui, kit, cx, c, images, wanted, actions);
                        }
                    });
                })
                .response
                .rect;
            ui.painter().line_segment(
                [
                    Pos2::new(r.left() + 3.0, r.top()),
                    Pos2::new(r.left() + 3.0, r.bottom()),
                ],
                Stroke::new(3.0, kit.color(Tok::SyntaxQuote)),
            );
        }
        BlockKind::List { start, items } => {
            for (i, item) in items.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.add_space(4.0);
                    match (item.task, start) {
                        (Some((done, range)), _) => {
                            let mut checked = done;
                            let resp = ui.add_enabled(
                                cx.interactive,
                                egui::Checkbox::without_text(&mut checked),
                            );
                            if resp.clicked() {
                                cmd(
                                    actions,
                                    "markdown.toggleTask",
                                    json!({ "doc": cx.doc.0, "start": range.0, "end": range.1 }),
                                );
                            }
                        }
                        (None, Some(n)) => {
                            ui.label(
                                RichText::new(format!("{}.", n + i as u64))
                                    .size(cx.size)
                                    .color(kit.color(Tok::SyntaxListMarker)),
                            );
                        }
                        (None, None) => {
                            ui.label(
                                RichText::new("•")
                                    .size(cx.size)
                                    .color(kit.color(Tok::SyntaxListMarker)),
                            );
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = cx.size * 0.3;
                        for c in &item.blocks {
                            block(ui, kit, cx, c, images, wanted, actions);
                        }
                    });
                });
            }
        }
        BlockKind::Table {
            aligns,
            header,
            rows,
        } => {
            // A table wider than the column scrolls sideways.
            egui::ScrollArea::horizontal()
                .id_salt(("md-table-scroll", b.range.0))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    egui::Grid::new(("md-table", b.range.0))
                        .striped(true)
                        .spacing([18.0, 6.0])
                        .show(ui, |ui| {
                            for (i, cell) in header.iter().enumerate() {
                                cell_ui(
                                    ui,
                                    kit,
                                    cx,
                                    cell,
                                    aligns.get(i).copied().unwrap_or(Align::None),
                                    true,
                                    images,
                                    wanted,
                                    actions,
                                );
                            }
                            ui.end_row();
                            for row in rows {
                                for (i, cell) in row.iter().enumerate() {
                                    cell_ui(
                                        ui,
                                        kit,
                                        cx,
                                        cell,
                                        aligns.get(i).copied().unwrap_or(Align::None),
                                        false,
                                        images,
                                        wanted,
                                        actions,
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
        }
        BlockKind::Rule => {
            ui.separator();
        }
        BlockKind::Html(src) => {
            ui.label(
                RichText::new(src.trim_end())
                    .font(crate::fonts::mono(cx.size * 0.85))
                    .color(kit.color(Tok::TextSecondary)),
            );
        }
        BlockKind::FrontMatter(src) => {
            egui::CollapsingHeader::new(
                RichText::new(t!("markdown:frontMatter")).size(cx.size * 0.9),
            )
            .id_salt(("front-matter", b.range.0))
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt(("fm-scroll", b.range.0))
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        egui::Grid::new(("fm", b.range.0)).show(ui, |ui| {
                            for line in src.lines().filter(|l| !l.trim().is_empty()) {
                                let (k, v) = line.split_once([':', '=']).unwrap_or((line, ""));
                                ui.label(
                                    RichText::new(k.trim())
                                        .font(crate::fonts::mono(cx.size * 0.85))
                                        .color(kit.color(Tok::SyntaxProperty)),
                                );
                                ui.label(
                                    RichText::new(v.trim())
                                        .font(crate::fonts::mono(cx.size * 0.85)),
                                );
                                ui.end_row();
                            }
                        });
                    });
            });
        }
        BlockKind::Footnote { label, blocks } => {
            ui.horizontal_top(|ui| {
                ui.label(
                    RichText::new(format!("[{label}]"))
                        .size(cx.size * 0.85)
                        .color(kit.color(Tok::TextTertiary)),
                );
                ui.vertical(|ui| {
                    for c in blocks {
                        block(ui, kit, cx, c, images, wanted, actions);
                    }
                });
            });
        }
    }
    let _ = text;
}

#[allow(clippy::too_many_arguments)]
fn cell_ui(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    cx: &Ctx,
    cell: &[Inline],
    align: Align,
    header: bool,
    images: &HashMap<PathBuf, ImageTex>,
    wanted: &mut Vec<PathBuf>,
    actions: &mut Actions,
) {
    let layout = match align {
        Align::Right => egui::Layout::right_to_left(egui::Align::Center),
        Align::Center => egui::Layout::top_down(egui::Align::Center),
        _ => egui::Layout::left_to_right(egui::Align::Center),
    };
    ui.with_layout(layout, |ui| {
        // Cells size to their content instead of wrapping per character.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        inline_flow(ui, kit, cx, cell, cx.size, header, images, wanted, actions);
    });
}

/// Text with `[[wiki links]]`: plain runs and `wiki:` link runs showing the
/// alias (or the target).
fn wiki_runs(text: &str, style: Style, out: &mut Vec<Run>) {
    let chars: Vec<char> = text.chars().collect();
    let mut last = 0;
    for l in notoir_editor::wiki::links(text) {
        if l.range.0 > last {
            out.push(Run::Text(
                chars[last..l.range.0].iter().collect(),
                style,
                None,
            ));
        }
        let target = match &l.heading {
            Some(h) => format!("{}#{h}", l.target),
            None => l.target.clone(),
        };
        let shown = l.alias.clone().unwrap_or_else(|| target.clone());
        out.push(Run::Text(shown, style, Some(format!("wiki:{target}"))));
        last = l.range.1;
    }
    if last < chars.len() {
        out.push(Run::Text(chars[last..].iter().collect(), style, None));
    }
}

/// One styled run of inline content.
enum Run {
    Text(String, Style, Option<String>),
    Image(String, String),
    Break,
}

fn flatten(inlines: &[Inline], style: Style, link: Option<&str>, out: &mut Vec<Run>) {
    for i in inlines {
        match i {
            Inline::Text(t) if link.is_none() && t.contains("[[") => wiki_runs(t, style, out),
            Inline::Text(t) => out.push(Run::Text(t.clone(), style, link.map(str::to_owned))),
            Inline::Html(t) => out.push(Run::Text(
                t.clone(),
                Style {
                    code: true,
                    ..style
                },
                link.map(str::to_owned),
            )),
            Inline::Code(t) => out.push(Run::Text(
                t.clone(),
                Style {
                    code: true,
                    ..style
                },
                link.map(str::to_owned),
            )),
            Inline::Emphasis(c) => flatten(
                c,
                Style {
                    emphasis: true,
                    ..style
                },
                link,
                out,
            ),
            Inline::Strong(c) => flatten(
                c,
                Style {
                    strong: true,
                    ..style
                },
                link,
                out,
            ),
            Inline::Strike(c) => flatten(
                c,
                Style {
                    strike: true,
                    ..style
                },
                link,
                out,
            ),
            Inline::Link { url, children, .. } => flatten(children, style, Some(url), out),
            Inline::Image { url, alt } => out.push(Run::Image(url.clone(), alt.clone())),
            Inline::FootnoteRef(l) => {
                out.push(Run::Text(format!("[{l}]"), style, Some(format!("#fn-{l}"))))
            }
            Inline::SoftBreak => out.push(Run::Text(" ".into(), style, None)),
            Inline::HardBreak => out.push(Run::Break),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn inline_flow(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    cx: &Ctx,
    inlines: &[Inline],
    size: f32,
    heading: bool,
    images: &HashMap<PathBuf, ImageTex>,
    wanted: &mut Vec<PathBuf>,
    actions: &mut Actions,
) {
    let mut runs = Vec::new();
    flatten(
        inlines,
        Style {
            strong: heading,
            ..Default::default()
        },
        None,
        &mut runs,
    );
    // Table cells (`Extend`) stay on one line; the table scrolls instead.
    let one_line = ui.style().wrap_mode == Some(egui::TextWrapMode::Extend);
    let flow = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for run in runs {
            match run {
                Run::Break => ui.end_row(),
                Run::Text(text, st, link) => {
                    let font = if st.code {
                        crate::fonts::mono(size * 0.9)
                    } else if st.strong {
                        crate::fonts::semibold(size)
                    } else {
                        FontId::proportional(size)
                    };
                    let missing = link.as_deref().is_some_and(|url| {
                        url.strip_prefix("wiki:").is_some_and(|t| {
                            let name = t.split('#').next().unwrap_or(t).trim();
                            !name.is_empty() && cx.missing.contains(&name.to_lowercase())
                        })
                    });
                    let color = if missing {
                        kit.color(Tok::Warning)
                    } else if link.is_some() {
                        kit.color(Tok::SyntaxLink)
                    } else if st.code {
                        kit.color(Tok::SyntaxCode)
                    } else {
                        kit.color(Tok::TextPrimary)
                    };
                    let code_bg = kit.color(Tok::BgPanel);
                    let rich = |piece: &str| {
                        let mut rt = RichText::new(piece).font(font.clone()).color(color);
                        if st.emphasis {
                            rt = rt.italics();
                        }
                        if st.strike {
                            rt = rt.strikethrough();
                        }
                        if st.code {
                            rt = rt.background_color(code_bg);
                        }
                        if link.is_some() && !missing {
                            rt = rt.underline();
                        }
                        rt
                    };
                    // A wiki link to a missing note: dashed, with an icon.
                    if missing {
                        ui.add(kit.icon(
                            ui.ctx(),
                            crate::icons::Icon::FilePlus,
                            size * 0.85,
                            kit.color(Tok::Warning),
                        ));
                    }
                    // One label per word: the row wraps between words, and a
                    // word longer than the column starts a row and wraps
                    // inside it instead of running past the edge.
                    for piece in text.split_inclusive(char::is_whitespace) {
                        let Some(url) = &link else {
                            ui.add(egui::Label::new(rich(piece)));
                            continue;
                        };
                        let resp = ui.add(egui::Label::new(rich(piece)).sense(Sense::click()));
                        let resp = if missing {
                            let r = resp.rect;
                            let mut x = r.left();
                            while x < r.right() {
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(x, r.bottom() - 1.0),
                                        egui::pos2((x + 3.0).min(r.right()), r.bottom() - 1.0),
                                    ],
                                    egui::Stroke::new(1.0, color),
                                );
                                x += 6.0;
                            }
                            resp.on_hover_text(t!("notes:missingHint"))
                        } else {
                            resp.on_hover_text(url)
                        };
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            cmd(
                                actions,
                                "markdown.openLink",
                                json!({ "doc": cx.doc.0, "url": url }),
                            );
                        }
                    }
                }
                Run::Image(url, alt) => image(ui, kit, cx, &url, &alt, images, wanted),
            }
        }
    };
    if one_line {
        ui.horizontal(flow);
    } else {
        ui.horizontal_wrapped(flow);
    }
}

fn image(
    ui: &mut egui::Ui,
    kit: &mut Kit,
    cx: &Ctx,
    url: &str,
    alt: &str,
    images: &HashMap<PathBuf, ImageTex>,
    wanted: &mut Vec<PathBuf>,
) {
    let lower = url.to_lowercase();
    let placeholder = |ui: &mut egui::Ui, kit: &mut Kit, text: String, button: Option<String>| {
        egui::Frame::new()
            .stroke(Stroke::new(1.0, kit.color(Tok::Border)))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(text)
                            .size(cx.size * 0.85)
                            .color(kit.color(Tok::TextTertiary)),
                    );
                    if let Some(b) = button {
                        // Remote loading needs network support (not in this build).
                        ui.add_enabled(false, egui::Button::new(b))
                            .on_disabled_hover_text(t!("markdown:remoteUnavailable"));
                    }
                });
            });
    };
    if lower.starts_with("http://") || lower.starts_with("https://") {
        // Privacy: never fetched by default (spec 24).
        let button = (cx.remote != "never").then(|| t!("markdown:loadRemote"));
        placeholder(ui, kit, t!("markdown:remoteImage", url = url), button);
        return;
    }
    let Some(base) = &cx.base_dir else {
        placeholder(ui, kit, t!("markdown:imageMissing", path = url), None);
        return;
    };
    let path = {
        let p = Path::new(url);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base.join(p)
        }
    };
    match images.get(&path) {
        Some(ImageTex::Ready(tex, size)) => {
            let max = ui.available_width().max(40.0);
            let scale = (max / size.x).min(1.0);
            ui.add(egui::Image::new((tex.id(), *size * scale)))
                .on_hover_text(alt);
        }
        Some(ImageTex::Missing(why)) => {
            placeholder(
                ui,
                kit,
                format!("{} — {why}", t!("markdown:imageMissing", path = url)),
                None,
            );
        }
        None => {
            wanted.push(path);
            ui.add(egui::Spinner::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec 24: "Code block highlighting"
    #[test]
    fn rust_code_blocks_use_syntax_colours() {
        let theme = crate::theme::ThemeId::Midnight.load();
        let job = code_job(
            &theme,
            "rust",
            "fn main() { let x = 42; }\n",
            FontId::monospace(13.0),
        );
        let colors: std::collections::HashSet<Color32> =
            job.sections.iter().map(|s| s.format.color).collect();
        assert!(colors.contains(&theme.color(Tok::SyntaxKeyword)));
        assert!(colors.contains(&theme.color(Tok::SyntaxNumber)));
        assert!(colors.contains(&theme.color(Tok::SyntaxFunction)));
        assert_eq!(job.text, "fn main() { let x = 42; }");
        let plain = code_job(&theme, "", "just text", FontId::monospace(13.0));
        assert!(
            plain
                .sections
                .iter()
                .all(|s| s.format.color == theme.color(Tok::EditorForeground))
        );
    }
}
