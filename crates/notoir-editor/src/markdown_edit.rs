//! Pure Markdown insertion operations. Positions are character offsets.
use crate::edit_ops::EditResult;
use crate::{ChangeSet, EditKind, Range, Rope, Selection};

#[derive(Clone, Debug)]
pub enum Insert<'a> {
    Heading(u8),
    Quote,
    Bullet,
    Numbered,
    Task,
    Code(&'a str),
    Table { columns: usize, rows: usize },
    Rule,
}

fn result(start: usize, end: usize, text: &str, content: Range) -> EditResult {
    EditResult {
        changes: ChangeSet::replace(start, end, text),
        selection: Selection::single(Range::new(start + content.anchor, start + content.head)),
        kind: EditKind::Other,
    }
}

/// Applies block formatting to the primary selection only.
pub fn insert(rope: &Rope, selection: &Selection, kind: Insert<'_>) -> Option<EditResult> {
    let r = selection.primary();
    let eol = if rope.line(0).to_string().ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let (a, b) = (r.from(), r.to());
    if b > rope.len_chars() {
        return None;
    }
    if matches!(
        kind,
        Insert::Heading(_) | Insert::Quote | Insert::Bullet | Insert::Numbered | Insert::Task
    ) {
        let first = rope.char_to_line(a);
        let mut last = rope.char_to_line(b);
        if b > a && b == rope.line_to_char(last) {
            last = last.saturating_sub(1);
        }
        let start = rope.line_to_char(first);
        let end = if last + 1 < rope.len_lines() {
            rope.line_to_char(last + 1)
        } else {
            rope.len_chars()
        };
        let mut text = String::new();
        let mut first_content = 0;
        for line in first..=last {
            let raw = rope.line(line).to_string();
            let body = raw.trim_end_matches(['\r', '\n']);
            let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
            let (space, mut body) = body.split_at(indent);
            let prefix = match kind {
                Insert::Heading(level @ 1..=6) => {
                    let hashes = body.bytes().take_while(|b| *b == b'#').count();
                    if (1..=6).contains(&hashes)
                        && (body.len() == hashes || body[hashes..].starts_with([' ', '\t']))
                    {
                        body = body[hashes..].trim_start_matches([' ', '\t']);
                    }
                    format!("{} ", "#".repeat(level as usize))
                }
                Insert::Quote => "> ".into(),
                Insert::Bullet => "- ".into(),
                Insert::Numbered => format!("{}. ", line - first + 1),
                Insert::Task => "- [ ] ".into(),
                _ => return None,
            };
            if line == first {
                first_content = space.chars().count() + prefix.chars().count();
            }
            text.push_str(space);
            text.push_str(&prefix);
            text.push_str(body);
            text.push_str(&raw[raw.trim_end_matches(['\r', '\n']).len()..]);
        }
        let content = if r.is_empty() {
            Range::caret(first_content)
        } else {
            Range::new(0, text.chars().count())
        };
        return Some(result(start, end, &text, content));
    }
    let selected = rope.slice(a..b).to_string();
    let (block, content) = match kind {
        Insert::Code(language) => {
            let longest = selected
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0);
            let fence = "`".repeat(3.max(longest + 1));
            let open = format!("{fence}{language}{eol}");
            let tail = if selected.is_empty() || selected.ends_with('\n') {
                ""
            } else {
                eol
            };
            let text = format!("{open}{selected}{tail}{fence}");
            let offset = open.chars().count();
            // An empty block needs a separate content line.
            let text = if selected.is_empty() {
                format!("{open}{eol}{fence}")
            } else {
                text
            };
            (text, Range::new(offset, offset + selected.chars().count()))
        }
        Insert::Table { columns, rows } => {
            if !(1..=100).contains(&columns) || !(1..=100).contains(&rows) {
                return None;
            }
            let cells = format!("|{}", "  |".repeat(columns));
            let delimiter = format!("|{}", " --- |".repeat(columns));
            let mut text = format!("{cells}{eol}{delimiter}");
            for _ in 0..rows {
                text.push_str(eol);
                text.push_str(&cells);
            }
            (text, Range::caret(2))
        }
        Insert::Rule => ("---".into(), Range::caret(3)),
        _ => return None,
    };
    // Only the adjacent newline boundaries are needed, even for large documents.
    let before = rope.slice(a.saturating_sub(4)..a).to_string();
    let after = rope.slice(b..(b + 4).min(rope.len_chars())).to_string();
    let leading = if before.is_empty() || before.ends_with(&format!("{eol}{eol}")) {
        String::new()
    } else if before.ends_with('\n') {
        eol.to_owned()
    } else {
        eol.repeat(2)
    };
    let trailing = if after.starts_with(&format!("{eol}{eol}")) {
        String::new()
    } else if after.starts_with(eol) {
        eol.to_owned()
    } else {
        eol.repeat(2)
    };
    let offset = leading.chars().count();
    let text = format!("{leading}{block}{trailing}");
    Some(result(
        a,
        b,
        &text,
        Range::new(offset + content.anchor, offset + content.head),
    ))
}

pub fn escape_caption(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}

/// Encode characters that could terminate or change an inline destination.
pub fn destination(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' ' => "%20".into(),
            '(' => "%28".into(),
            ')' => "%29".into(),
            '<' => "%3C".into(),
            '>' => "%3E".into(),
            '\\' => "%5C".into(),
            c if c.is_control() => c.to_string().bytes().map(|b| format!("%{b:02X}")).collect(),
            c => c.to_string(),
        })
        .collect()
}

/// Decode a local Markdown destination once, preserving malformed encodings.
pub fn decode_destination(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(a), Some(b)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            decoded.push((a * 16 + b) as u8);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

pub fn link(range: Range, caption: &str, url: &str, image: bool) -> EditResult {
    let text = format!(
        "{}[{}]({})",
        if image { "!" } else { "" },
        escape_caption(caption),
        destination(url)
    );
    let caret = text.chars().count();
    result(range.from(), range.to(), &text, Range::caret(caret))
}

/// A parsed link selected for editing. Offsets are characters in `source`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExistingLink {
    pub range: Range,
    pub caption: String,
    pub destination: String,
    raw_caption: String,
    title: String,
}

/// Find one unambiguous link at a caret or within a selection. Parsing keeps
/// escaped Markdown, images and code spans from being mistaken for links.
pub fn existing_link(source: &str, selection: Range) -> Option<ExistingLink> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let rope = Rope::from_str(source);
    let mut open = None;
    let mut found = Vec::new();
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                open = Some((
                    range.start,
                    range.end,
                    dest_url.to_string(),
                    title.to_string(),
                    String::new(),
                    None::<std::ops::Range<usize>>,
                ));
            }
            Event::End(TagEnd::Link) => {
                let Some((start, end, destination, title, caption, content)) = open.take() else {
                    continue;
                };
                let end = end.max(range.end);
                let span = Range::new(rope.byte_to_char(start), rope.byte_to_char(end));
                if (span.from() <= selection.from() && selection.to() <= span.to())
                    || (selection.from() <= span.from() && span.to() <= selection.to())
                {
                    let raw_caption = content.map_or_else(String::new, |r| source[r].to_owned());
                    found.push(ExistingLink {
                        range: span,
                        caption,
                        destination,
                        raw_caption,
                        title,
                    });
                }
            }
            event => {
                if let Some((_, _, _, _, caption, content)) = &mut open {
                    if let Some(content) = content {
                        content.start = content.start.min(range.start);
                        content.end = content.end.max(range.end);
                    } else {
                        *content = Some(range);
                    }
                    match event {
                        Event::Text(text) | Event::Code(text) => caption.push_str(&text),
                        Event::SoftBreak | Event::HardBreak => caption.push(' '),
                        _ => {}
                    }
                }
            }
        }
    }
    (found.len() == 1).then(|| found.remove(0))
}

/// Update a link, retaining its title and caption markup when only its URL
/// changes. An edited caption uses the same escaping as Insert Link.
pub fn update_link(current: &ExistingLink, caption: &str, url: &str) -> EditResult {
    let caption = if caption == current.caption {
        current.raw_caption.clone()
    } else {
        escape_caption(caption)
    };
    let title = if current.title.is_empty() {
        String::new()
    } else {
        format!(
            " \"{}\"",
            current
                .title
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace(['\r', '\n'], " ")
        )
    };
    let text = format!("[{caption}]({}{title})", destination(url));
    result(
        current.range.from(),
        current.range.to(),
        &text,
        Range::caret(text.chars().count()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_link_prefills_and_updates_without_nesting() {
        let raw = "before [**café** \\[notes\\]](https://old.test \"Title\") after";
        let current = existing_link(raw, Range::new(0, raw.chars().count())).unwrap();
        assert_eq!(current.caption, "café [notes]");
        assert_eq!(current.destination, "https://old.test");
        let edit = update_link(&current, &current.caption, "https://new.test/a b");
        let mut rope = Rope::from_str(raw);
        edit.changes.apply(&mut rope);
        assert_eq!(
            rope.to_string(),
            "before [**café** \\[notes\\]](https://new.test/a%20b \"Title\") after"
        );
        let edit = update_link(&current, "New [caption]", "https://new.test");
        let mut rope = Rope::from_str(raw);
        edit.changes.apply(&mut rope);
        assert_eq!(
            rope.to_string(),
            "before [New \\[caption\\]](https://new.test \"Title\") after"
        );
    }

    #[test]
    fn link_detection_uses_the_caret_and_rejects_code_images_and_ambiguity() {
        let raw = "α [one](https://one.test) [two](https://two.test)";
        assert_eq!(
            existing_link(raw, Range::caret(5)).unwrap().destination,
            "https://one.test"
        );
        assert!(existing_link(raw, Range::new(0, raw.chars().count())).is_none());
        for raw in [
            "`[caption](https://test)`",
            "![caption](https://test)",
            "\\[caption](https://test)",
            "plain text",
        ] {
            assert!(
                existing_link(raw, Range::new(0, raw.chars().count())).is_none(),
                "{raw}"
            );
        }
    }
    fn apply(text: &str, r: Range, kind: Insert<'_>) -> (String, Selection) {
        let mut rope = Rope::from_str(text);
        let edit = insert(&rope, &Selection::single(r), kind).unwrap();
        edit.changes.apply(&mut rope);
        (rope.to_string(), edit.selection)
    }
    #[test]
    fn headings_replace_prefix_and_exclude_final_line() {
        assert_eq!(
            apply("## café\r\nnext", Range::new(0, 9), Insert::Heading(1)).0,
            "# café\r\nnext"
        );
        assert!(insert(&Rope::new(), &Selection::caret(0), Insert::Heading(7)).is_none());
    }
    #[test]
    fn line_templates_and_numbering() {
        for (kind, expected) in [
            (Insert::Quote, "> "),
            (Insert::Bullet, "- "),
            (Insert::Task, "- [ ] "),
        ] {
            let (text, sel) = apply("", Range::caret(0), kind);
            assert_eq!(text, expected);
            assert_eq!(sel.primary().head, expected.chars().count());
        }
        assert_eq!(
            apply("a\nb\nc", Range::new(0, 4), Insert::Numbered).0,
            "1. a\n2. b\nc"
        );
    }
    #[test]
    fn safe_fences_and_block_boundaries() {
        assert_eq!(
            apply("a```b", Range::new(0, 5), Insert::Code("rust")).0,
            "````rust\na```b\n````\n\n"
        );
        let (text, sel) = apply("", Range::caret(0), Insert::Code(""));
        assert_eq!(text, "```\n\n```\n\n");
        assert_eq!(sel.primary().head, 4);
        assert_eq!(
            apply("ab", Range::caret(1), Insert::Rule).0,
            "a\n\n---\n\nb"
        );
    }
    #[test]
    fn table_dimensions_render_with_a_header_and_requested_data_rows() {
        for (columns, rows) in [(1, 1), (3, 4), (5, 2)] {
            let (text, selection) = apply("", Range::caret(0), Insert::Table { columns, rows });
            let parsed = crate::markdown::parse(&text);
            let crate::markdown::BlockKind::Table {
                header, rows: body, ..
            } = &parsed.blocks[0].kind
            else {
                panic!("expected table");
            };
            assert_eq!(header.len(), columns);
            assert_eq!(body.len(), rows);
            assert!(body.iter().all(|row| row.len() == columns));
            assert_eq!(selection.primary().head, 2);
        }
        for (columns, rows) in [(0, 1), (1, 0), (101, 1), (1, 101)] {
            assert!(
                insert(
                    &Rope::new(),
                    &Selection::caret(0),
                    Insert::Table { columns, rows }
                )
                .is_none()
            );
        }
    }

    #[test]
    fn local_destinations_decode_once() {
        assert_eq!(decode_destination("a%20%23%25%28x%29.md"), "a #%(x).md");
        assert_eq!(decode_destination("%2520.md"), "%20.md");
        assert_eq!(decode_destination("caf%C3%A9.md"), "café.md");
        assert_eq!(decode_destination("a%xx.md"), "a%xx.md");
    }

    #[test]
    fn link_escaping() {
        let e = link(Range::caret(0), "[café]", "a b(x).md", false);
        let mut rope = Rope::new();
        e.changes.apply(&mut rope);
        assert_eq!(rope.to_string(), "[\\[café\\]](a%20b%28x%29.md)");
    }
}
