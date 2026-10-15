//! Outline providers (docs/specs/26 §Outline): Markdown headings nested by
//! level; JSON / YAML / TOML keys to a depth. Code has no symbols until a
//! symbol extractor exists.

use ropey::Rope;

use crate::display_map::line_text;
use crate::json;
use crate::language::LanguageId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Heading(u8),
    Key,
    Table,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Char range of the whole symbol (section, key and value).
    pub range: (usize, usize),
    /// Char range to select / reveal (the heading line, the key).
    pub selection_range: (usize, usize),
    pub children: Vec<Symbol>,
}

/// Symbols of a document; `depth` limits key nesting (headings are not limited).
pub fn symbols(rope: &Rope, language: LanguageId, depth: usize) -> Vec<Symbol> {
    match language.as_str() {
        "markdown" => markdown(rope),
        "json" | "jsonc" | "jsonl" => json_symbols(rope, language, depth),
        "yaml" => yaml(rope, depth),
        "toml" => toml(rope, depth),
        _ => Vec::new(),
    }
}

/// The chain of symbols containing `offset`, outermost first.
pub fn path_at(symbols: &[Symbol], offset: usize) -> Vec<&Symbol> {
    let mut out = Vec::new();
    let mut level = symbols;
    while let Some(s) = level
        .iter()
        .rev()
        .find(|s| s.range.0 <= offset && offset <= s.range.1)
    {
        out.push(s);
        level = &s.children;
    }
    out
}

/// Every symbol, depth-first, with its depth.
pub fn flatten(symbols: &[Symbol]) -> Vec<(usize, &Symbol)> {
    fn walk<'a>(s: &'a [Symbol], d: usize, out: &mut Vec<(usize, &'a Symbol)>) {
        for x in s {
            out.push((d, x));
            walk(&x.children, d + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(symbols, 0, &mut out);
    out
}

fn markdown(rope: &Rope) -> Vec<Symbol> {
    use crate::markdown::{BlockKind, parse, plain_text};
    let text = rope.to_string();
    let doc = parse(&text);
    let len = rope.len_chars();
    let heads: Vec<(u8, String, (usize, usize))> = doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Heading { level, inlines, .. } => {
                let line = rope.char_to_line(b.range.0.min(len));
                let line_end = rope.line_to_char(line) + line_text(rope, line).chars().count();
                Some((*level, plain_text(inlines), (b.range.0, line_end)))
            }
            _ => None,
        })
        .collect();
    // Section ends: the next heading of the same or a higher level.
    let flat: Vec<(u8, Symbol)> = heads
        .iter()
        .enumerate()
        .map(|(i, (level, name, sel))| {
            let end = heads[i + 1..]
                .iter()
                .find(|h| h.0 <= *level)
                .map_or(len, |h| h.2.0.saturating_sub(1).max(sel.1));
            (
                *level,
                Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Heading(*level),
                    range: (sel.0, end),
                    selection_range: *sel,
                    children: Vec::new(),
                },
            )
        })
        .collect();
    nest(flat)
}

/// Builds a tree from (level, symbol) in document order.
fn nest(flat: Vec<(u8, Symbol)>) -> Vec<Symbol> {
    let mut roots: Vec<Symbol> = Vec::new();
    // Stack of (level, path of indices).
    let mut stack: Vec<(u8, Vec<usize>)> = Vec::new();
    for (level, sym) in flat {
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            stack.pop();
        }
        let path = match stack.last() {
            None => {
                roots.push(sym);
                vec![roots.len() - 1]
            }
            Some((_, p)) => {
                let mut node = &mut roots[p[0]];
                for &i in &p[1..] {
                    node = &mut node.children[i];
                }
                node.children.push(sym);
                let mut q = p.clone();
                q.push(node.children.len() - 1);
                q
            }
        };
        stack.push((level, path));
    }
    roots
}

fn json_symbols(rope: &Rope, language: LanguageId, depth: usize) -> Vec<Symbol> {
    let Some(dialect) = json::Dialect::for_language(language.as_str()) else {
        return Vec::new();
    };
    let doc = json::parse(&rope.to_string(), dialect);
    fn conv(n: &json::JsonNode, depth: usize) -> Vec<Symbol> {
        if depth == 0 {
            return Vec::new();
        }
        n.children
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let sel = c.key_range.unwrap_or(c.range);
                Symbol {
                    name: c.key.clone().unwrap_or_else(|| format!("[{i}]")),
                    kind: SymbolKind::Key,
                    range: (sel.0, c.range.1),
                    selection_range: sel,
                    children: conv(c, depth - 1),
                }
            })
            .collect()
    }
    doc.roots.iter().flat_map(|r| conv(r, depth)).collect()
}

fn line_start(rope: &Rope, l: usize) -> usize {
    rope.line_to_char(l)
}

/// `key:` lines nested by indentation.
fn yaml(rope: &Rope, depth: usize) -> Vec<Symbol> {
    let mut flat = Vec::new();
    let mut indents: Vec<usize> = Vec::new();
    for l in 0..rope.len_lines() {
        let t = line_text(rope, l);
        let trimmed = t.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }
        let indent = t.len() - trimmed.len();
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let key = trimmed[..colon].trim().trim_matches(['"', '\'']);
        if key.is_empty() || key.contains(' ') && !trimmed.starts_with(['"', '\'']) {
            continue;
        }
        while indents.last().is_some_and(|&i| i >= indent) {
            indents.pop();
        }
        indents.push(indent);
        if indents.len() > depth {
            continue;
        }
        let start = line_start(rope, l) + indent;
        let key_len = key.chars().count();
        flat.push((
            indents.len() as u8,
            l,
            Symbol {
                name: key.to_owned(),
                kind: SymbolKind::Key,
                range: (start, start),
                selection_range: (start, start + key_len),
                children: Vec::new(),
            },
        ));
    }
    close_ranges(rope, flat)
}

/// Section ranges end before the next symbol of the same or a higher level.
fn close_ranges(rope: &Rope, mut flat: Vec<(u8, usize, Symbol)>) -> Vec<Symbol> {
    let len = rope.len_chars();
    for i in 0..flat.len() {
        let level = flat[i].0;
        let end = flat[i + 1..]
            .iter()
            .find(|x| x.0 <= level)
            .map_or(len, |x| line_start(rope, x.1).saturating_sub(1));
        flat[i].2.range.1 = end.max(flat[i].2.selection_range.1);
    }
    nest(flat.into_iter().map(|(l, _, s)| (l, s)).collect())
}

/// `[table]` / `[[array]]` headers (full dotted name) with their keys.
fn toml(rope: &Rope, depth: usize) -> Vec<Symbol> {
    let mut flat = Vec::new();
    let mut in_table = 0usize;
    for l in 0..rope.len_lines() {
        let t = line_text(rope, l);
        let trimmed = t.trim();
        let start = line_start(rope, l) + (t.len() - t.trim_start().len());
        if trimmed.starts_with('[') {
            let name = trimmed
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim();
            let level = 1;
            in_table = level;
            if depth == 0 {
                continue;
            }
            flat.push((
                level as u8,
                l,
                Symbol {
                    name: name.to_owned(),
                    kind: SymbolKind::Table,
                    range: (start, start),
                    selection_range: (start, start + trimmed.chars().count()),
                    children: Vec::new(),
                },
            ));
        } else if let Some(eq) = trimmed.find('=')
            && !trimmed.starts_with('#')
            && in_table < depth
        {
            let key = trimmed[..eq].trim().trim_matches(['"', '\'']);
            if key.is_empty() {
                continue;
            }
            flat.push((
                (in_table + 1) as u8,
                l,
                Symbol {
                    name: key.to_owned(),
                    kind: SymbolKind::Key,
                    range: (start, start),
                    selection_range: (start, start + key.chars().count()),
                    children: Vec::new(),
                },
            ));
        }
    }
    close_ranges(rope, flat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &[Symbol]) -> Vec<String> {
        s.iter().map(|x| x.name.clone()).collect()
    }

    #[test]
    fn markdown_headings_nest() {
        let src = "# Introduction\n\n# Architecture\n\n## Editor\n\n## Git\n\n## MCP\n\nmcp text\n\n# Roadmap\n\nSetext\n------\n";
        let rope = Rope::from_str(src);
        let s = symbols(&rope, LanguageId::MARKDOWN, 2);
        assert_eq!(names(&s), ["Introduction", "Architecture", "Roadmap"]);
        assert_eq!(names(&s[1].children), ["Editor", "Git", "MCP"]);
        assert_eq!(names(&s[2].children), ["Setext"]);
        let at = src.find("mcp text").unwrap();
        let chain: Vec<_> = path_at(&s, at).iter().map(|x| x.name.clone()).collect();
        assert_eq!(chain, ["Architecture", "MCP"]);
        assert_eq!(
            s[1].children[2].selection_range.0,
            src.find("## MCP").unwrap()
        );
    }

    #[test]
    fn json_keys_to_depth() {
        let src = r#"{"editor":{"minimap":{"enabled":true}},"tags":[1]}"#;
        let rope = Rope::from_str(src);
        let s = symbols(&rope, LanguageId::JSON, 2);
        assert_eq!(names(&s), ["editor", "tags"]);
        assert_eq!(names(&s[0].children), ["minimap"]);
        assert!(s[0].children[0].children.is_empty());
        let full = symbols(&rope, LanguageId::JSON, usize::MAX);
        let at = src.find("true").unwrap() + 1;
        let chain: Vec<_> = path_at(&full, at).iter().map(|x| x.name.clone()).collect();
        assert_eq!(chain, ["editor", "minimap", "enabled"]);
    }

    #[test]
    fn yaml_and_toml() {
        let y = Rope::from_str("name: x\nserver:\n  port: 1\n  tls:\n    on: true\n");
        let s = symbols(&y, LanguageId("yaml"), 2);
        assert_eq!(names(&s), ["name", "server"]);
        assert_eq!(names(&s[1].children), ["port", "tls"]);
        let t = Rope::from_str(
            "title = 1\n[package]\nname = \"a\"\n[dependencies.serde]\nversion = 1\n",
        );
        let s = symbols(&t, LanguageId("toml"), 2);
        assert_eq!(names(&s), ["title", "package", "dependencies.serde"]);
        assert_eq!(names(&s[1].children), ["name"]);
    }
}
