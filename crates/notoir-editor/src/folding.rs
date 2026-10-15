//! Folding providers (docs/specs/12 §Folding): Markdown sections, code blocks
//! and lists; bracket pairs for JSON and brace languages (skipping strings and
//! comments); indentation for everything else; `#region` markers anywhere.

use ropey::Rope;

use crate::display_map::line_text;
use crate::language::LanguageId;

/// A foldable region: `start_line` stays visible, `start_line + 1 ..=
/// end_line` hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start_line: usize,
    pub end_line: usize,
    /// Nesting depth, 1 = outermost (a JSON document's root is 0).
    pub level: u8,
}

/// Fold ranges for `rope` in `language`. `large` (large-file mode) uses the
/// indentation strategy only.
pub fn fold_ranges(rope: &Rope, language: LanguageId, large: bool) -> Vec<FoldRange> {
    let lang = language.as_str();
    let mut raw: Vec<(usize, usize)> = if large {
        indentation(rope)
    } else {
        match lang {
            "markdown" => markdown(rope),
            "json" | "jsonc" | "jsonl" | "javascript" | "typescript" | "rust" | "css" | "c"
            | "cpp" | "java" | "csharp" | "go" | "swift" | "kotlin" | "php" | "scss" | "less" => {
                brackets(rope, lang)
            }
            _ => indentation(rope),
        }
    };
    raw.extend(regions(rope));
    let mut out = finish(raw);
    if matches!(lang, "json" | "jsonc") && !large {
        // The root value is level 0: "Fold Level 1" folds top-level values.
        let first = (0..rope.len_lines()).find(|&l| !line_text(rope, l).trim().is_empty());
        if out.first().is_some_and(|r| Some(r.start_line) == first) {
            let root_end = out[0].end_line;
            for r in &mut out {
                if r.end_line <= root_end {
                    r.level = r.level.saturating_sub(1);
                }
            }
        }
    }
    out
}

/// Sorts, drops ranges that hide nothing, keeps one range per start line (the
/// largest) and assigns nesting levels.
fn finish(mut raw: Vec<(usize, usize)>) -> Vec<FoldRange> {
    raw.retain(|r| r.1 > r.0);
    raw.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    raw.dedup_by_key(|r| r.0);
    let mut out = Vec::with_capacity(raw.len());
    let mut stack: Vec<usize> = Vec::new();
    for (s, e) in raw {
        while stack.last().is_some_and(|&end| s > end) {
            stack.pop();
        }
        // A range that overlaps its parent's end is clipped inside it.
        let e = stack.last().map_or(e, |&end| e.min(end));
        if e <= s {
            continue;
        }
        stack.push(e);
        out.push(FoldRange {
            start_line: s,
            end_line: e,
            level: stack.len().min(255) as u8,
        });
    }
    out
}

fn last_content_line(rope: &Rope, from: usize, to: usize) -> usize {
    let mut e = to;
    while e > from && line_text(rope, e).trim().is_empty() {
        e -= 1;
    }
    e
}

fn markdown(rope: &Rope) -> Vec<(usize, usize)> {
    use crate::markdown::{BlockKind, parse};
    let text = rope.to_string();
    let doc = parse(&text);
    let last = rope.len_lines().saturating_sub(1);
    let line_of = |c: usize| rope.char_to_line(c.min(rope.len_chars()));
    // The line a block ends on (ranges are exclusive; a trailing newline
    // belongs to the previous line).
    let end_line_of = |c: usize| line_of(c.saturating_sub(1));
    let mut out = Vec::new();
    let headings: Vec<(usize, u8)> = doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Heading { level, .. } => Some((line_of(b.range.0), *level)),
            _ => None,
        })
        .collect();
    for (i, &(line, level)) in headings.iter().enumerate() {
        let next = headings[i + 1..]
            .iter()
            .find(|(_, l)| *l <= level)
            .map_or(last + 1, |(nl, _)| *nl);
        out.push((line, last_content_line(rope, line, next.saturating_sub(1))));
    }
    fn blocks(
        bs: &[crate::markdown::Block],
        out: &mut Vec<(usize, usize)>,
        line_of: &dyn Fn(usize) -> usize,
        end_of: &dyn Fn(usize) -> usize,
    ) {
        for b in bs {
            match &b.kind {
                BlockKind::Code { .. } | BlockKind::FrontMatter(_) | BlockKind::Table { .. } => {
                    out.push((line_of(b.range.0), end_of(b.range.1)));
                }
                BlockKind::Quote(inner) => {
                    out.push((line_of(b.range.0), end_of(b.range.1)));
                    blocks(inner, out, line_of, end_of);
                }
                BlockKind::List { items, .. } => {
                    for it in items {
                        out.push((line_of(it.range.0), end_of(it.range.1)));
                        blocks(&it.blocks, out, line_of, end_of);
                    }
                }
                _ => {}
            }
        }
    }
    blocks(&doc.blocks, &mut out, &line_of, &end_line_of);
    out
}

fn brackets(rope: &Rope, lang: &str) -> Vec<(usize, usize)> {
    let hash_comments = false;
    let single_quotes = !matches!(lang, "json" | "jsonc" | "jsonl" | "rust");
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut in_block = false;
    for line in 0..rope.len_lines() {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        let mut i = 0;
        let mut quote: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if in_block {
                if c == '*' && chars.get(i + 1) == Some(&'/') {
                    in_block = false;
                    i += 1;
                }
                i += 1;
                continue;
            }
            if let Some(q) = quote {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            match c {
                '/' if chars.get(i + 1) == Some(&'/') => break,
                '#' if hash_comments => break,
                '/' if chars.get(i + 1) == Some(&'*') => {
                    in_block = true;
                    i += 1;
                }
                '"' | '`' => quote = Some(c),
                '\'' if single_quotes => quote = Some(c),
                // Rust: a char literal, not a lifetime.
                '\'' if lang == "rust"
                    && (chars.get(i + 2) == Some(&'\'') || chars.get(i + 1) == Some(&'\\')) =>
                {
                    quote = Some(c)
                }
                '{' | '[' | '(' => stack.push(line),
                '}' | ']' | ')' => {
                    if let Some(start) = stack.pop()
                        && line > start
                    {
                        // The closing line stays visible when it only closes.
                        let rest: String = chars[..i].iter().collect();
                        let end = if rest.trim().is_empty() {
                            line - 1
                        } else {
                            line
                        };
                        out.push((start, end));
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    out
}

fn indent_of(s: &str) -> Option<usize> {
    if s.trim().is_empty() {
        return None;
    }
    Some(
        s.chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum(),
    )
}

fn indentation(rope: &Rope) -> Vec<(usize, usize)> {
    let n = rope.len_lines();
    let indents: Vec<Option<usize>> = (0..n).map(|l| indent_of(&line_text(rope, l))).collect();
    let mut out = Vec::new();
    // Stack of (line, indent) of open headers.
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut last_content = 0;
    for (l, ind) in indents.iter().enumerate() {
        let Some(ind) = *ind else { continue };
        while let Some(&(sl, si)) = stack.last() {
            if ind <= si {
                stack.pop();
                out.push((sl, last_content));
            } else {
                break;
            }
        }
        stack.push((l, ind));
        last_content = l;
    }
    for (sl, _) in stack {
        out.push((sl, last_content));
    }
    out
}

/// `#region` / `// region` … `#endregion` markers.
fn regions(rope: &Rope) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for l in 0..rope.len_lines() {
        let t = line_text(rope, l);
        let t = t
            .trim_start()
            .trim_start_matches(['/', '#', '-', '*', '!', '<', ';'])
            .trim_start();
        let t = t.trim_start_matches('#');
        if t.starts_with("endregion") {
            if let Some(s) = stack.pop() {
                out.push((s, l));
            }
        } else if t.starts_with("region") {
            stack.push(l);
        }
    }
    out
}

/// The fold whose header is `line`, or else the innermost one containing it.
pub fn range_at(ranges: &[FoldRange], line: usize) -> Option<FoldRange> {
    if let Some(r) = ranges.iter().find(|r| r.start_line == line) {
        return Some(*r);
    }
    ranges
        .iter()
        .filter(|r| r.start_line < line && line <= r.end_line)
        .max_by_key(|r| r.start_line)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(r: &[FoldRange]) -> Vec<(usize, usize, u8)> {
        r.iter()
            .map(|f| (f.start_line, f.end_line, f.level))
            .collect()
    }

    #[test]
    fn markdown_sections() {
        let rope = Rope::from_str("# Architecture\ntext\n## Editor\nmore\n# Roadmap\n");
        let r = fold_ranges(&rope, LanguageId::MARKDOWN, false);
        assert!(lines(&r).contains(&(0, 3, 1)), "{r:?}");
        assert!(lines(&r).contains(&(2, 3, 2)));
        let rope = Rope::from_str("# A\n\n```rust\nfn x() {}\n```\n\n- a\n  - b\n");
        let r = fold_ranges(&rope, LanguageId::MARKDOWN, false);
        assert!(
            r.iter().any(|f| f.start_line == 2 && f.end_line == 4),
            "{r:?}"
        );
    }

    #[test]
    fn json_brackets_and_levels() {
        let src = "{\n  \"a\": {\n    \"x\": 1\n  },\n  \"b\": [\n    1, \"]\"\n  ]\n}\n";
        let r = fold_ranges(&Rope::from_str(src), LanguageId::JSON, false);
        assert_eq!(lines(&r), vec![(0, 6, 0), (1, 2, 1), (4, 5, 1)]);
    }

    #[test]
    fn rust_braces_skip_strings_comments_lifetimes() {
        let src = "fn f<'a>(x: &'a str) {\n    let s = \"{\";\n    // }\n    let c = '}';\n}\n";
        let r = fold_ranges(&Rope::from_str(src), LanguageId("rust"), false);
        assert_eq!(lines(&r), vec![(0, 3, 1)]);
    }

    #[test]
    fn indentation_and_regions() {
        let src = "a:\n  b: 1\n  c:\n    d: 2\n\ne: 3\n# region x\nf\n# endregion\n";
        let r = fold_ranges(&Rope::from_str(src), LanguageId("yaml"), false);
        assert!(lines(&r).contains(&(0, 3, 1)), "{r:?}");
        assert!(lines(&r).contains(&(2, 3, 2)));
        assert!(lines(&r).contains(&(6, 8, 1)));
        assert_eq!(range_at(&r, 3).map(|f| f.start_line), Some(2));
    }
}
