//! Syntax highlighting (docs/specs/26 §Syntax Highlighting): syntect grammars
//! (bat's extended set via `two-face`, precompiled, pure-Rust regex) mapped to
//! Notoir's syntax tokens, highlighted incrementally per line.
//!
//! Each line keeps the parser state at its start and its spans. An edit
//! re-highlights from the first changed line and stops as soon as the state
//! at the start of a later line equals the one cached before the edit, so a
//! keystroke usually costs a few lines. Highlighting is driven up to a target
//! line within a time budget; callers continue on later frames.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ropey::Rope;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use crate::language::{LanguageId, definition};

/// Token classes the theme colours (spec 21 syntax tokens).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Plain,
    Comment,
    Keyword,
    String,
    Number,
    Constant,
    Function,
    Type,
    Variable,
    Property,
    Operator,
    Punctuation,
    Tag,
    Attribute,
    Regex,
    Escape,
    Heading,
    Emphasis,
    Strong,
    Link,
    Code,
    Quote,
    ListMarker,
    Invalid,
}

/// A highlighted run within one line: char offsets [start, end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        syntect::dumps::from_uncompressed_data(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/syntaxes.packdump"
        )))
        .expect("bundled syntax packdump")
    })
}

/// Grammar names for languages whose syntect name differs from the display
/// name; others match by display name or extension.
fn grammar_name(lang: LanguageId) -> Option<&'static str> {
    Some(match lang.as_str() {
        "jsonc" | "jsonl" => "JSON",
        "shellscript" => "Bourne Again Shell (bash)",
        "gitcommit" => "Git Commit",
        "dotenv" => "DotENV",
        "log" => "log",
        "plaintext" => return None,
        _ => return None,
    })
}

fn syntax_for(lang: LanguageId) -> Option<&'static SyntaxReference> {
    if lang == LanguageId::PLAINTEXT {
        return None;
    }
    let ss = syntax_set();
    let def = definition(lang);
    grammar_name(lang)
        .and_then(|n| ss.find_syntax_by_name(n))
        .or_else(|| ss.find_syntax_by_name(def.name))
        .or_else(|| {
            def.extensions
                .iter()
                .find_map(|e| ss.find_syntax_by_extension(e))
        })
        .or_else(|| {
            def.file_names
                .iter()
                .find_map(|f| ss.find_syntax_by_extension(f))
        })
}

/// Whether a language has a grammar.
pub fn has_grammar(lang: LanguageId) -> bool {
    syntax_for(lang).is_some()
}

/// Innermost scope wins; a few broad scopes are checked by prefix.
fn classify(stack: &ScopeStack) -> TokenKind {
    for scope in stack.as_slice().iter().rev() {
        let s = scope.build_string();
        let k = |p: &str| s == p || s.starts_with(&format!("{p}."));
        let kind = if k("comment") || k("punctuation.definition.comment") {
            TokenKind::Comment
        } else if k("constant.character.escape") {
            TokenKind::Escape
        } else if k("string.regexp") {
            TokenKind::Regex
        } else if k("string") || k("punctuation.definition.string") {
            TokenKind::String
        } else if k("constant.numeric") {
            TokenKind::Number
        } else if k("constant") || k("support.constant") {
            TokenKind::Constant
        } else if k("markup.heading") || k("entity.name.section") {
            TokenKind::Heading
        } else if k("markup.bold") {
            TokenKind::Strong
        } else if k("markup.italic") {
            TokenKind::Emphasis
        } else if k("markup.underline.link") || k("markup.link") || k("meta.link") {
            TokenKind::Link
        } else if k("markup.raw") || k("markup.inline.raw") {
            TokenKind::Code
        } else if k("markup.quote") {
            TokenKind::Quote
        } else if k("punctuation.definition.list_item")
            || k("markup.list.numbered.bullet")
            || k("markup.list.unnumbered.bullet")
        {
            TokenKind::ListMarker
        } else if k("invalid") {
            TokenKind::Invalid
        } else if k("keyword.operator") {
            TokenKind::Operator
        } else if k("keyword") || k("storage") {
            TokenKind::Keyword
        } else if k("entity.name.function")
            || k("support.function")
            || k("variable.function")
            || k("meta.function-call.identifier")
        {
            TokenKind::Function
        } else if k("entity.name.type")
            || k("entity.name.class")
            || k("entity.name.struct")
            || k("entity.name.enum")
            || k("support.type")
            || k("support.class")
            || k("entity.other.inherited-class")
        {
            TokenKind::Type
        } else if k("entity.name.tag") {
            TokenKind::Tag
        } else if k("entity.other.attribute-name") {
            TokenKind::Attribute
        } else if k("support.type.property-name")
            || k("meta.mapping.key")
            || k("variable.other.property")
            || k("variable.other.member")
            || k("entity.name.key")
        {
            TokenKind::Property
        } else if k("variable") {
            TokenKind::Variable
        } else if k("punctuation") {
            TokenKind::Punctuation
        } else {
            continue;
        };
        return kind;
    }
    TokenKind::Plain
}

#[derive(Clone)]
struct LineState {
    start: (ParseState, ScopeStack),
    spans: Option<Arc<Vec<Span>>>,
}

/// Incremental highlighter for one buffer.
#[derive(Clone)]
pub struct Highlighter {
    /// Per line; `None` = start state unknown yet.
    lines: Vec<Option<LineState>>,
    /// Lines `[0, valid)` have spans matching the buffer.
    valid: usize,
    /// After an edit: re-highlighting may stop at a line `>= settle_from`
    /// whose cached start state matches, and then everything up to
    /// `resume_valid` is valid again.
    settle_from: usize,
    resume_valid: usize,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter")
            .field("lines", &self.lines.len())
            .field("valid", &self.valid)
            .finish()
    }
}

impl Highlighter {
    /// `None` for plain text or languages without a grammar.
    pub fn new(lang: LanguageId, rope: &Rope) -> Option<Highlighter> {
        let syntax = syntax_for(lang)?;
        let mut lines = vec![None; rope.len_lines()];
        if let Some(first) = lines.first_mut() {
            *first = Some(LineState {
                start: (ParseState::new(syntax), ScopeStack::new()),
                spans: None,
            });
        }
        Some(Highlighter {
            lines,
            valid: 0,
            settle_from: 0,
            resume_valid: 0,
        })
    }

    /// Lines highlighted so far from the top.
    pub fn valid_lines(&self) -> usize {
        self.valid
    }

    /// Spans of a line when it is highlighted.
    pub fn spans(&self, line: usize) -> Option<&Arc<Vec<Span>>> {
        if line >= self.valid {
            return None;
        }
        self.lines.get(line)?.as_ref()?.spans.as_ref()
    }

    /// Lines `[first, first + removed]` of the old buffer became
    /// `[first, first + inserted]`.
    pub fn edited(&mut self, first: usize, removed: usize, inserted: usize) {
        let first = first.min(self.lines.len().saturating_sub(1));
        let old_end = (first + removed + 1).min(self.lines.len());
        let keep_start = self
            .lines
            .get(first)
            .and_then(|l| l.as_ref().map(|l| l.start.clone()));
        let fresh = std::iter::repeat_n(None, inserted + 1);
        self.lines.splice(first..old_end, fresh);
        if let (Some(start), Some(slot)) = (keep_start, self.lines.get_mut(first)) {
            *slot = Some(LineState { start, spans: None });
        }
        let delta = inserted as isize - removed as isize;
        let after_edit = first + inserted + 1;
        let old_valid = self.valid;
        if old_valid > first + removed + 1 {
            self.resume_valid = ((old_valid as isize) + delta).max(0) as usize;
            self.settle_from = after_edit;
        } else {
            self.resume_valid = 0;
            self.settle_from = usize::MAX;
        }
        self.valid = self.valid.min(first);
    }

    /// Highlights until `target` (inclusive) is valid or `deadline` passes.
    /// Returns true when `target` is done.
    pub fn highlight_to(&mut self, rope: &Rope, target: usize, deadline: Instant) -> bool {
        let total = rope.len_lines();
        if self.lines.len() != total {
            // Out of sync (should not happen): start over.
            self.lines.resize(total, None);
            self.valid = 0;
        }
        let target = target.min(total.saturating_sub(1));
        let ss = syntax_set();
        let mut checks = 0u32;
        while self.valid <= target && self.valid < total {
            let i = self.valid;
            let Some(Some(line)) = self.lines.get(i).cloned() else {
                return false;
            };
            let (mut state, mut stack) = line.start.clone();
            let text: String = rope.line(i).chars().collect();
            let ops = state.parse_line(&text, ss).unwrap_or_default();
            // Byte → char offsets for this line.
            let byte_to_char = |b: usize| text[..b.min(text.len())].chars().count();
            let mut spans: Vec<Span> = Vec::new();
            let mut last = 0usize;
            let content_end = text.trim_end_matches(['\n', '\r']).len();
            let push = |from: usize, to: usize, kind: TokenKind, spans: &mut Vec<Span>| {
                let (from, to) = (from.min(content_end), to.min(content_end));
                if to > from && kind != TokenKind::Plain {
                    let (a, b) = (byte_to_char(from), byte_to_char(to));
                    match spans.last_mut() {
                        Some(s) if s.end == a && s.kind == kind => s.end = b,
                        _ => spans.push(Span {
                            start: a,
                            end: b,
                            kind,
                        }),
                    }
                }
            };
            for (pos, op) in ops {
                push(last, pos, classify(&stack), &mut spans);
                let _ = stack.apply(&op);
                last = pos;
            }
            push(last, text.len(), classify(&stack), &mut spans);
            if let Some(Some(l)) = self.lines.get_mut(i) {
                l.spans = Some(Arc::new(spans));
            }
            let next = (state, stack);
            self.valid = i + 1;
            if i + 1 < total {
                // Converged with the state cached before the edit: the rest
                // is still right.
                let cached_same = self
                    .lines
                    .get(i + 1)
                    .and_then(|l| l.as_ref())
                    .is_some_and(|l| l.start == next && l.spans.is_some());
                if i + 1 >= self.settle_from && cached_same && self.resume_valid > i + 1 {
                    self.valid = self.resume_valid.min(total);
                    self.settle_from = usize::MAX;
                    continue;
                }
                self.lines[i + 1] = Some(LineState {
                    start: next,
                    spans: None,
                });
            }
            checks += 1;
            if checks.is_multiple_of(16) && Instant::now() >= deadline {
                return self.valid > target;
            }
        }
        self.valid > target || self.valid >= total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    fn kinds(h: &Highlighter, rope: &Rope, line: usize) -> Vec<(String, TokenKind)> {
        let text: Vec<char> = rope.line(line).chars().collect();
        h.spans(line)
            .unwrap()
            .iter()
            .map(|s| (text[s.start..s.end].iter().collect(), s.kind))
            .collect()
    }

    #[test]
    fn rust_tokens() {
        let rope = Rope::from_str("// note\nfn main() { let x = \"hi\"; 42 }\n");
        let mut h = Highlighter::new(LanguageId::from_id("rust").unwrap(), &rope).unwrap();
        assert!(h.highlight_to(&rope, 10, far()));
        assert_eq!(
            kinds(&h, &rope, 0)[0],
            ("// note".into(), TokenKind::Comment)
        );
        let l1 = kinds(&h, &rope, 1);
        assert!(l1.contains(&("fn".into(), TokenKind::Keyword)), "{l1:?}");
        assert!(l1.contains(&("main".into(), TokenKind::Function)), "{l1:?}");
        assert!(
            l1.iter()
                .any(|(t, k)| t.contains("hi") && *k == TokenKind::String),
            "{l1:?}"
        );
        assert!(l1.contains(&("42".into(), TokenKind::Number)), "{l1:?}");
    }

    // spec 26: "TOML and TypeScript are highlighted"
    #[test]
    fn toml_and_typescript_have_grammars() {
        let toml = Rope::from_str("[package]\nname = \"notoir\"\n");
        let mut h = Highlighter::new(LanguageId::from_id("toml").unwrap(), &toml).unwrap();
        h.highlight_to(&toml, 5, far());
        assert!(
            kinds(&h, &toml, 1)
                .iter()
                .any(|(_, k)| *k == TokenKind::String)
        );
        let ts = Rope::from_str("const n: number = 1;\n");
        let mut h = Highlighter::new(LanguageId::from_id("typescript").unwrap(), &ts).unwrap();
        h.highlight_to(&ts, 5, far());
        let k = kinds(&h, &ts, 0);
        assert!(k.contains(&("const".into(), TokenKind::Keyword)), "{k:?}");
        assert!(
            k.iter().any(|(_, kind)| *kind == TokenKind::Number),
            "{k:?}"
        );
        assert!(Highlighter::new(LanguageId::PLAINTEXT, &ts).is_none());
    }

    #[test]
    fn every_language_has_a_grammar() {
        let missing: Vec<&str> = crate::language::LANGUAGES
            .iter()
            .filter(|l| l.id != LanguageId::PLAINTEXT && !has_grammar(l.id))
            .map(|l| l.id.as_str())
            .collect();
        assert!(missing.is_empty(), "no grammar for {missing:?}");
    }

    fn rust_file(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("let v{i} = {i}; // line {i}\n"))
            .collect()
    }

    // spec 26: "Highlighting updates incrementally"
    #[test]
    fn edits_rehighlight_incrementally_within_budget() {
        let mut rope = Rope::from_str(&rust_file(10_000));
        let mut h = Highlighter::new(LanguageId::from_id("rust").unwrap(), &rope).unwrap();
        assert!(h.highlight_to(&rope, 10_000, far()));
        assert_eq!(h.valid_lines(), rope.len_lines());
        // A quote on line 5,000 opens a string that runs to the end.
        let at = rope.line_to_char(4999) + 4;
        rope.insert(at, "\"");
        h.edited(4999, 0, 0);
        assert_eq!(h.valid_lines(), 4999);
        // The viewport (lines up to 5,040) is ready within a frame budget…
        let start = Instant::now();
        assert!(h.highlight_to(&rope, 5040, Instant::now() + Duration::from_millis(8)));
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "{:?}",
            start.elapsed()
        );
        assert!(
            kinds(&h, &rope, 5001)
                .iter()
                .all(|(_, k)| *k == TokenKind::String),
            "rest of file is inside the string"
        );
        // …and the rest was not re-highlighted synchronously.
        assert!(h.valid_lines() < 9000, "{}", h.valid_lines());
        // Closing it again converges quickly with the cached states.
        rope.insert(rope.line_to_char(4999) + 4, "\"");
        h.edited(4999, 0, 0);
        assert!(h.highlight_to(&rope, 5040, far()));
    }

    #[test]
    fn convergence_keeps_states_after_a_local_edit() {
        let mut rope = Rope::from_str(&rust_file(2_000));
        let mut h = Highlighter::new(LanguageId::from_id("rust").unwrap(), &rope).unwrap();
        h.highlight_to(&rope, 2_000, far());
        // Change a number on line 10: line 11 onwards stays valid.
        let at = rope.line_to_char(10) + 8;
        rope.insert(at, "7");
        h.edited(10, 0, 0);
        assert!(h.highlight_to(&rope, 10, far()));
        assert_eq!(
            h.valid_lines(),
            rope.len_lines(),
            "converged after one line"
        );
        // Inserting lines shifts the cache.
        let at = rope.line_to_char(20);
        rope.insert(at, "// a\n// b\n");
        h.edited(20, 0, 2);
        h.highlight_to(&rope, 25, far());
        assert_eq!(h.valid_lines(), rope.len_lines());
        assert_eq!(kinds(&h, &rope, 21)[0].1, TokenKind::Comment);
        assert!(kinds(&h, &rope, 22).contains(&("let".into(), TokenKind::Keyword)));
    }
}
