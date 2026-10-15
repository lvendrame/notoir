//! Markdown render tree (docs/specs/24 §Preview Rendering): pulldown-cmark
//! (CommonMark + GFM tables, task lists, strikethrough, footnotes, heading
//! attributes, front matter) parsed into blocks and inlines that keep their
//! source ranges, so the preview can map back to the text (scroll sync, task
//! toggles, double-click to source).

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};

/// Char range in the source.
pub type SrcRange = (usize, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    Link {
        url: String,
        title: String,
        children: Vec<Inline>,
    },
    Image {
        url: String,
        alt: String,
    },
    FootnoteRef(String),
    /// Inline HTML is shown as text (no HTML engine).
    Html(String),
    SoftBreak,
    HardBreak,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// Task state and the source range of its `[ ]` / `[x]` marker.
    pub task: Option<(bool, SrcRange)>,
    pub blocks: Vec<Block>,
    pub range: SrcRange,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        slug: String,
    },
    Paragraph(Vec<Inline>),
    Code {
        lang: String,
        text: String,
    },
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Table {
        aligns: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
    /// HTML blocks are shown as escaped source.
    Html(String),
    /// YAML/TOML front matter, shown as a collapsible table.
    FrontMatter(String),
    Footnote {
        label: String,
        blocks: Vec<Block>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub range: SrcRange,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MdDoc {
    pub blocks: Vec<Block>,
}

/// GitHub-style heading anchor: lowercase, spaces → `-`, punctuation dropped.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' || c == '_' {
            out.push(if c == '_' { '_' } else { '-' });
        }
    }
    out
}

/// Plain text of inlines (heading slugs, alt text).
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Html(t) => s.push_str(t),
            Inline::Emphasis(c)
            | Inline::Strong(c)
            | Inline::Strike(c)
            | Inline::Link { children: c, .. } => s.push_str(&plain_text(c)),
            Inline::Image { alt, .. } => s.push_str(alt),
            Inline::FootnoteRef(l) => s.push_str(l),
            Inline::SoftBreak | Inline::HardBreak => s.push(' '),
        }
    }
    s
}

enum Frame {
    Blocks(Vec<Block>),
    Inlines(Vec<Inline>),
}

/// Byte → char offsets for one source string.
struct Offsets {
    starts: Vec<usize>,
}

impl Offsets {
    fn new(s: &str) -> Self {
        Self {
            starts: s.char_indices().map(|(b, _)| b).collect(),
        }
    }

    fn char(&self, byte: usize) -> usize {
        self.starts.partition_point(|&b| b < byte)
    }

    fn range(&self, r: std::ops::Range<usize>) -> SrcRange {
        (self.char(r.start), self.char(r.end))
    }
}

/// Parses Markdown into the render tree.
pub fn parse(source: &str) -> MdDoc {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    let off = Offsets::new(source);
    // Containers being built: (start tag info, frame, range).
    struct Open {
        tag: TagInfo,
        frame: Frame,
        range: SrcRange,
        task: Option<(bool, SrcRange)>,
        // Tables: rows of cells collected so far.
        table_rows: Vec<Vec<Vec<Inline>>>,
        table_head: Vec<Vec<Inline>>,
        items: Vec<ListItem>,
        text: String,
    }
    #[derive(Clone)]
    enum TagInfo {
        Root,
        Paragraph,
        Heading(u8),
        Quote,
        Code(String),
        List(Option<u64>),
        Item,
        Table(Vec<Align>),
        Head,
        Row,
        Cell,
        Emphasis,
        Strong,
        Strike,
        Link(String, String),
        Image(String),
        Footnote(String),
        Html,
        Meta,
        Other,
    }
    let mut stack: Vec<Open> = vec![Open {
        tag: TagInfo::Root,
        frame: Frame::Blocks(Vec::new()),
        range: (0, off.starts.len()),
        task: None,
        table_rows: Vec::new(),
        table_head: Vec::new(),
        items: Vec::new(),
        text: String::new(),
    }];
    let new_open = |tag: TagInfo, inline: bool, range: SrcRange| Open {
        tag,
        frame: if inline {
            Frame::Inlines(Vec::new())
        } else {
            Frame::Blocks(Vec::new())
        },
        range,
        task: None,
        table_rows: Vec::new(),
        table_head: Vec::new(),
        items: Vec::new(),
        text: String::new(),
    };
    fn push_inline(stack: &mut [Open], i: Inline) {
        if let Some(top) = stack.last_mut() {
            match &mut top.frame {
                Frame::Inlines(v) => v.push(i),
                Frame::Blocks(v) => {
                    // Loose text directly in a block container (tight list
                    // items): wrap in a paragraph.
                    match v.last_mut() {
                        Some(Block {
                            kind: BlockKind::Paragraph(p),
                            ..
                        }) => p.push(i),
                        _ => v.push(Block {
                            kind: BlockKind::Paragraph(vec![i]),
                            range: top.range,
                        }),
                    }
                }
            }
        }
    }
    fn push_block(stack: &mut [Open], b: Block) {
        if let Some(top) = stack.last_mut()
            && let Frame::Blocks(v) = &mut top.frame
        {
            v.push(b);
        }
    }
    for (ev, r) in Parser::new_ext(source, opts).into_offset_iter() {
        let range = off.range(r);
        match ev {
            Event::Start(tag) => {
                let (info, inline) = match tag {
                    Tag::Paragraph => (TagInfo::Paragraph, true),
                    Tag::Heading { level, .. } => (
                        TagInfo::Heading(match level {
                            HeadingLevel::H1 => 1,
                            HeadingLevel::H2 => 2,
                            HeadingLevel::H3 => 3,
                            HeadingLevel::H4 => 4,
                            HeadingLevel::H5 => 5,
                            HeadingLevel::H6 => 6,
                        }),
                        true,
                    ),
                    Tag::BlockQuote(_) => (TagInfo::Quote, false),
                    Tag::CodeBlock(kind) => (
                        TagInfo::Code(match kind {
                            CodeBlockKind::Fenced(l) => {
                                l.split_whitespace().next().unwrap_or("").to_owned()
                            }
                            CodeBlockKind::Indented => String::new(),
                        }),
                        true,
                    ),
                    Tag::HtmlBlock => (TagInfo::Html, true),
                    Tag::List(start) => (TagInfo::List(start), false),
                    Tag::Item => (TagInfo::Item, false),
                    Tag::FootnoteDefinition(l) => (TagInfo::Footnote(l.to_string()), false),
                    Tag::Table(a) => (
                        TagInfo::Table(
                            a.iter()
                                .map(|a| match a {
                                    Alignment::None => Align::None,
                                    Alignment::Left => Align::Left,
                                    Alignment::Center => Align::Center,
                                    Alignment::Right => Align::Right,
                                })
                                .collect(),
                        ),
                        false,
                    ),
                    Tag::TableHead => (TagInfo::Head, false),
                    Tag::TableRow => (TagInfo::Row, false),
                    Tag::TableCell => (TagInfo::Cell, true),
                    Tag::Emphasis => (TagInfo::Emphasis, true),
                    Tag::Strong => (TagInfo::Strong, true),
                    Tag::Strikethrough => (TagInfo::Strike, true),
                    Tag::Link {
                        dest_url, title, ..
                    } => (TagInfo::Link(dest_url.to_string(), title.to_string()), true),
                    Tag::Image { dest_url, .. } => (TagInfo::Image(dest_url.to_string()), true),
                    Tag::MetadataBlock(_) => (TagInfo::Meta, true),
                    _ => (TagInfo::Other, true),
                };
                stack.push(new_open(info, inline, range));
            }
            Event::End(end) => {
                let Some(open) = stack.pop() else { continue };
                let (inlines, children) = match open.frame {
                    Frame::Inlines(v) => (v, Vec::new()),
                    Frame::Blocks(v) => (Vec::new(), v),
                };
                let _ = end;
                match open.tag {
                    TagInfo::Paragraph => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::Paragraph(inlines),
                            range: open.range,
                        },
                    ),
                    TagInfo::Heading(level) => {
                        let slug = slug(&plain_text(&inlines));
                        push_block(
                            &mut stack,
                            Block {
                                kind: BlockKind::Heading {
                                    level,
                                    inlines,
                                    slug,
                                },
                                range: open.range,
                            },
                        );
                    }
                    TagInfo::Code(lang) => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::Code {
                                lang,
                                text: open.text,
                            },
                            range: open.range,
                        },
                    ),
                    TagInfo::Html => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::Html(open.text),
                            range: open.range,
                        },
                    ),
                    TagInfo::Meta => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::FrontMatter(open.text),
                            range: open.range,
                        },
                    ),
                    TagInfo::Quote => {
                        push_block(
                            &mut stack,
                            Block {
                                kind: BlockKind::Quote(children),
                                range: open.range,
                            },
                        );
                    }
                    TagInfo::Item => {
                        let item = ListItem {
                            task: open.task,
                            blocks: children,
                            range: open.range,
                        };
                        if let Some(top) = stack.last_mut() {
                            top.items.push(item);
                        }
                    }
                    TagInfo::List(start) => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::List {
                                start,
                                items: open.items,
                            },
                            range: open.range,
                        },
                    ),
                    TagInfo::Footnote(label) => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::Footnote {
                                label,
                                blocks: children,
                            },
                            range: open.range,
                        },
                    ),
                    TagInfo::Table(aligns) => push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::Table {
                                aligns,
                                header: open.table_head,
                                rows: open.table_rows,
                            },
                            range: open.range,
                        },
                    ),
                    TagInfo::Head => {
                        if let Some(top) = stack.last_mut() {
                            top.table_head = open.table_rows.into_iter().next().unwrap_or_default();
                        }
                    }
                    TagInfo::Row => {
                        if let Some(top) = stack.last_mut() {
                            let cells = open.table_rows.into_iter().next().unwrap_or_default();
                            top.table_rows.push(cells);
                        }
                    }
                    TagInfo::Cell => {
                        if let Some(top) = stack.last_mut() {
                            if top.table_rows.is_empty() {
                                top.table_rows.push(Vec::new());
                            }
                            top.table_rows[0].push(inlines);
                        }
                    }
                    TagInfo::Emphasis => push_inline(&mut stack, Inline::Emphasis(inlines)),
                    TagInfo::Strong => push_inline(&mut stack, Inline::Strong(inlines)),
                    TagInfo::Strike => push_inline(&mut stack, Inline::Strike(inlines)),
                    TagInfo::Link(url, title) => push_inline(
                        &mut stack,
                        Inline::Link {
                            url,
                            title,
                            children: inlines,
                        },
                    ),
                    TagInfo::Image(url) => {
                        let alt = plain_text(&inlines);
                        push_inline(&mut stack, Inline::Image { url, alt });
                    }
                    TagInfo::Root | TagInfo::Other => {
                        for i in inlines {
                            push_inline(&mut stack, i);
                        }
                    }
                }
            }
            Event::Text(t) => {
                let top_tag = stack.last().map(|o| o.tag.clone());
                match top_tag {
                    Some(TagInfo::Code(_) | TagInfo::Html | TagInfo::Meta) => {
                        if let Some(top) = stack.last_mut() {
                            top.text.push_str(&t);
                        }
                    }
                    _ => push_inline(&mut stack, Inline::Text(t.to_string())),
                }
            }
            Event::Code(t) => push_inline(&mut stack, Inline::Code(t.to_string())),
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                push_inline(&mut stack, Inline::Code(t.to_string()))
            }
            Event::Html(t) => {
                if let Some(top) = stack.last_mut()
                    && matches!(top.tag, TagInfo::Html)
                {
                    top.text.push_str(&t);
                } else {
                    push_block(
                        &mut stack,
                        Block {
                            kind: BlockKind::Html(t.to_string()),
                            range,
                        },
                    );
                }
            }
            Event::InlineHtml(t) => push_inline(&mut stack, Inline::Html(t.to_string())),
            Event::FootnoteReference(l) => {
                push_inline(&mut stack, Inline::FootnoteRef(l.to_string()))
            }
            Event::SoftBreak => push_inline(&mut stack, Inline::SoftBreak),
            Event::HardBreak => push_inline(&mut stack, Inline::HardBreak),
            Event::Rule => push_block(
                &mut stack,
                Block {
                    kind: BlockKind::Rule,
                    range,
                },
            ),
            Event::TaskListMarker(done) => {
                if let Some(item) = stack
                    .iter_mut()
                    .rev()
                    .find(|o| matches!(o.tag, TagInfo::Item))
                {
                    item.task = Some((done, range));
                }
            }
        }
    }
    let root = stack.into_iter().next().map(|o| o.frame);
    MdDoc {
        blocks: match root {
            Some(Frame::Blocks(v)) => v,
            _ => Vec::new(),
        },
    }
}

/// The heading whose slug is `slug` (relative links `b.md#setup`).
pub fn find_heading<'a>(doc: &'a MdDoc, slug: &str) -> Option<&'a Block> {
    fn walk<'a>(blocks: &'a [Block], slug: &str) -> Option<&'a Block> {
        for b in blocks {
            match &b.kind {
                BlockKind::Heading { slug: s, .. } if s == slug => return Some(b),
                BlockKind::Quote(c) | BlockKind::Footnote { blocks: c, .. } => {
                    if let Some(h) = walk(c, slug) {
                        return Some(h);
                    }
                }
                BlockKind::List { items, .. } => {
                    for i in items {
                        if let Some(h) = walk(&i.blocks, slug) {
                            return Some(h);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(&doc.blocks, slug)
}

/// Toggles the task marker at `marker` (`[ ]` ↔ `[x]`): returns the new
/// marker text for that range.
pub fn toggled_marker(current: &str) -> &'static str {
    if current.contains('x') || current.contains('X') {
        "[ ]"
    } else {
        "[x]"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_inlines_and_ranges() {
        let src = "# Intro\n\nSome **bold** and *it* with `code` and [link](https://x.y \"T\").\n\n> quote\n> more\n\n---\n";
        let doc = parse(src);
        assert_eq!(doc.blocks.len(), 4);
        match &doc.blocks[0].kind {
            BlockKind::Heading { level, slug, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(slug, "intro");
            }
            k => panic!("{k:?}"),
        }
        assert_eq!(doc.blocks[0].range, (0, 8));
        let BlockKind::Paragraph(p) = &doc.blocks[1].kind else {
            panic!()
        };
        assert!(p.contains(&Inline::Strong(vec![Inline::Text("bold".into())])));
        assert!(p.contains(&Inline::Code("code".into())));
        assert!(p.iter().any(
            |i| matches!(i, Inline::Link { url, title, .. } if url == "https://x.y" && title == "T")
        ));
        let BlockKind::Quote(q) = &doc.blocks[2].kind else {
            panic!()
        };
        assert_eq!(q.len(), 1);
        assert!(matches!(doc.blocks[3].kind, BlockKind::Rule));
    }

    #[test]
    fn lists_tasks_and_markers() {
        let src = "- [ ] buy milk\n- [x] done\n  - nested\n1. one\n2. two\n";
        let doc = parse(src);
        let BlockKind::List { start: None, items } = &doc.blocks[0].kind else {
            panic!("{:?}", doc.blocks)
        };
        assert_eq!(items.len(), 2);
        let (done, r) = items[0].task.unwrap();
        assert!(!done);
        assert_eq!(&src[r.0..r.1], "[ ]");
        assert!(items[1].task.unwrap().0);
        assert!(
            items[1]
                .blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::List { .. })),
            "nested list"
        );
        assert!(matches!(
            doc.blocks[1].kind,
            BlockKind::List { start: Some(1), .. }
        ));
        assert_eq!(toggled_marker("[ ]"), "[x]");
        assert_eq!(toggled_marker("[X]"), "[ ]");
    }

    #[test]
    fn tables_code_footnotes_front_matter() {
        let src = "---\ntitle: x\n---\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n```rust\nfn x() {}\n```\n\nNote[^1].\n\n[^1]: The note.\n";
        let doc = parse(src);
        assert!(matches!(&doc.blocks[0].kind, BlockKind::FrontMatter(t) if t.contains("title")));
        let BlockKind::Table {
            aligns,
            header,
            rows,
        } = &doc.blocks[1].kind
        else {
            panic!("{:?}", doc.blocks[1])
        };
        assert_eq!(aligns, &vec![Align::Left, Align::Right]);
        assert_eq!(plain_text(&header[0]), "a");
        assert_eq!(plain_text(&rows[0][1]), "2");
        assert!(
            matches!(&doc.blocks[2].kind, BlockKind::Code { lang, text } if lang == "rust" && text == "fn x() {}\n")
        );
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(&b.kind, BlockKind::Footnote { label, .. } if label == "1"))
        );
    }

    #[test]
    fn unicode_ranges_are_chars() {
        let src = "çã\n\n## Olá Mundo\n";
        let doc = parse(src);
        assert_eq!(doc.blocks[1].range.0, 4);
        assert!(find_heading(&doc, "olá-mundo").is_some());
    }
}
