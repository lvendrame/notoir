//! JSON support (docs/specs/25): a span-preserving recursive-descent parser
//! for strict JSON, JSONC (comments, trailing commas) and JSON Lines, with
//! error locations; and token-based Format / Minify / Sort Keys that keep key
//! order (and comments when formatting).

/// Char range in the source.
pub type SrcRange = (usize, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Json,
    /// Comments and trailing commas allowed.
    Jsonc,
    /// One value per line (`.jsonl`, `.ndjson`).
    Lines,
}

impl Dialect {
    /// The dialect for a language id.
    pub fn for_language(lang: &str) -> Option<Dialect> {
        match lang {
            "json" => Some(Dialect::Json),
            "jsonc" => Some(Dialect::Jsonc),
            "jsonl" => Some(Dialect::Lines),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JsonNode {
    pub kind: Kind,
    /// Property name (objects' children only).
    pub key: Option<String>,
    pub key_range: Option<SrcRange>,
    /// The value's range (brackets included).
    pub range: SrcRange,
    /// Scalars: the decoded string, or the literal text of numbers/bools/null.
    pub value: Option<String>,
    pub children: Vec<JsonNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// Char offset of the problem.
    pub offset: usize,
    /// 1-based.
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsonDoc {
    pub roots: Vec<JsonNode>,
    pub error: Option<JsonError>,
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    pos: usize,
    jsonc: bool,
    /// Byte → char conversion.
    chars: Vec<usize>,
}

type PResult<T> = Result<T, (usize, String)>;

impl<'a> Parser<'a> {
    fn new(src: &'a str, jsonc: bool) -> Self {
        Self {
            s: src.as_bytes(),
            src,
            pos: 0,
            jsonc,
            chars: src.char_indices().map(|(b, _)| b).collect(),
        }
    }

    fn ch(&self, byte: usize) -> usize {
        self.chars.partition_point(|&b| b < byte)
    }

    fn err<T>(&self, msg: &str) -> PResult<T> {
        Err((self.pos, msg.to_owned()))
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_ws(&mut self) -> PResult<()> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'/') if self.jsonc => match self.s.get(self.pos + 1) {
                    Some(b'/') => {
                        while self.peek().is_some_and(|c| c != b'\n') {
                            self.pos += 1;
                        }
                    }
                    Some(b'*') => {
                        let start = self.pos;
                        self.pos += 2;
                        loop {
                            match self.peek() {
                                None => {
                                    self.pos = start;
                                    return self.err("unterminated comment");
                                }
                                Some(b'*') if self.s.get(self.pos + 1) == Some(&b'/') => {
                                    self.pos += 2;
                                    break;
                                }
                                _ => self.pos += 1,
                            }
                        }
                    }
                    _ => return Ok(()),
                },
                _ => return Ok(()),
            }
        }
    }

    fn string(&mut self) -> PResult<String> {
        // At the opening quote.
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                self.pos = start;
                return self.err("unterminated string");
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let esc = self.s.get(self.pos + 1).copied();
                    self.pos += 2;
                    match esc {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{8}'),
                        Some(b'f') => out.push('\u{c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let hex = self.src.get(self.pos..self.pos + 4).unwrap_or("");
                            match u32::from_str_radix(hex, 16) {
                                Ok(n) => {
                                    out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                                    self.pos += 4;
                                }
                                Err(_) => return self.err("invalid \\u escape"),
                            }
                        }
                        _ => {
                            self.pos -= 1;
                            return self.err("invalid escape");
                        }
                    }
                }
                b'\n' => return self.err("unterminated string"),
                _ => {
                    // Copy one UTF-8 char.
                    let rest = &self.src[self.pos..];
                    let c = rest.chars().next().expect("non-empty");
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn value(&mut self, key: Option<(String, SrcRange)>) -> PResult<JsonNode> {
        self.skip_ws()?;
        let start = self.pos;
        let (key, key_range) = match key {
            Some((k, r)) => (Some(k), Some(r)),
            None => (None, None),
        };
        let node = |kind, value: Option<String>, children, p: &Parser, end: usize| JsonNode {
            kind,
            key: key.clone(),
            key_range,
            range: (p.ch(start), p.ch(end)),
            value,
            children,
        };
        match self.peek() {
            Some(b'{') => {
                self.pos += 1;
                let mut children = Vec::new();
                self.skip_ws()?;
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(node(Kind::Object, None, children, self, self.pos));
                }
                loop {
                    self.skip_ws()?;
                    if self.peek() != Some(b'"') {
                        if self.peek() == Some(b'}') && self.jsonc && !children.is_empty() {
                            self.pos += 1;
                            return Ok(node(Kind::Object, None, children, self, self.pos));
                        }
                        return self.err("expected property name");
                    }
                    let ks = self.pos;
                    let k = self.string()?;
                    let kr = (self.ch(ks), self.ch(self.pos));
                    self.skip_ws()?;
                    if self.peek() != Some(b':') {
                        return self.err("expected ':'");
                    }
                    self.pos += 1;
                    children.push(self.value(Some((k, kr)))?);
                    self.skip_ws()?;
                    match self.peek() {
                        Some(b',') => {
                            self.pos += 1;
                            self.skip_ws()?;
                            if self.peek() == Some(b'}') && !self.jsonc {
                                return self.err("trailing comma not allowed");
                            }
                        }
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(node(Kind::Object, None, children, self, self.pos));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut children = Vec::new();
                self.skip_ws()?;
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(node(Kind::Array, None, children, self, self.pos));
                }
                loop {
                    self.skip_ws()?;
                    if self.peek() == Some(b']') && self.jsonc && !children.is_empty() {
                        self.pos += 1;
                        return Ok(node(Kind::Array, None, children, self, self.pos));
                    }
                    children.push(self.value(None)?);
                    self.skip_ws()?;
                    match self.peek() {
                        Some(b',') => {
                            self.pos += 1;
                            self.skip_ws()?;
                            if self.peek() == Some(b']') && !self.jsonc {
                                return self.err("trailing comma not allowed");
                            }
                        }
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(node(Kind::Array, None, children, self, self.pos));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => {
                let v = self.string()?;
                Ok(node(Kind::String, Some(v), Vec::new(), self, self.pos))
            }
            Some(b't') if self.s[self.pos..].starts_with(b"true") => {
                self.pos += 4;
                Ok(node(
                    Kind::Bool,
                    Some("true".into()),
                    Vec::new(),
                    self,
                    self.pos,
                ))
            }
            Some(b'f') if self.s[self.pos..].starts_with(b"false") => {
                self.pos += 5;
                Ok(node(
                    Kind::Bool,
                    Some("false".into()),
                    Vec::new(),
                    self,
                    self.pos,
                ))
            }
            Some(b'n') if self.s[self.pos..].starts_with(b"null") => {
                self.pos += 4;
                Ok(node(
                    Kind::Null,
                    Some("null".into()),
                    Vec::new(),
                    self,
                    self.pos,
                ))
            }
            Some(c) if c == b'-' || c.is_ascii_digit() => {
                let s = self.pos;
                if c == b'-' {
                    self.pos += 1;
                }
                let digits = |p: &mut Parser| {
                    let d = p.pos;
                    while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                        p.pos += 1;
                    }
                    p.pos > d
                };
                if !digits(self) {
                    return self.err("invalid number");
                }
                if self.peek() == Some(b'.') {
                    self.pos += 1;
                    if !digits(self) {
                        return self.err("invalid number");
                    }
                }
                if matches!(self.peek(), Some(b'e' | b'E')) {
                    self.pos += 1;
                    if matches!(self.peek(), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                    if !digits(self) {
                        return self.err("invalid number");
                    }
                }
                let text = self.src[s..self.pos].to_owned();
                Ok(node(Kind::Number, Some(text), Vec::new(), self, self.pos))
            }
            None => self.err("unexpected end of input"),
            _ => self.err("expected a value"),
        }
    }
}

fn locate(src: &str, char_offset: usize, message: String) -> JsonError {
    let mut line = 1;
    let mut column = 1;
    for c in src.chars().take(char_offset) {
        if c == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    JsonError {
        offset: char_offset,
        line,
        column,
        message,
    }
}

fn parse_one(src: &str, jsonc: bool, base_chars: usize, full: &str) -> Result<JsonNode, JsonError> {
    let mut p = Parser::new(src, jsonc);
    let result = p.value(None).and_then(|n| {
        p.skip_ws()?;
        if p.pos < p.s.len() {
            return p.err("unexpected characters after the value");
        }
        Ok(n)
    });
    match result {
        Ok(mut n) => {
            if base_chars > 0 {
                shift(&mut n, base_chars);
            }
            Ok(n)
        }
        Err((byte, msg)) => Err(locate(full, base_chars + p.ch(byte), msg)),
    }
}

fn shift(n: &mut JsonNode, by: usize) {
    n.range = (n.range.0 + by, n.range.1 + by);
    if let Some(k) = &mut n.key_range {
        *k = (k.0 + by, k.1 + by);
    }
    for c in &mut n.children {
        shift(c, by);
    }
}

/// Parses a document in `dialect`.
pub fn parse(src: &str, dialect: Dialect) -> JsonDoc {
    match dialect {
        Dialect::Lines => {
            let mut doc = JsonDoc::default();
            let mut base = 0;
            for line in src.split_inclusive('\n') {
                let n = line.chars().count();
                if !line.trim().is_empty() {
                    match parse_one(line.trim_end_matches(['\n', '\r']), false, base, src) {
                        Ok(node) => doc.roots.push(node),
                        Err(e) => {
                            doc.error.get_or_insert(e);
                        }
                    }
                }
                base += n;
            }
            doc
        }
        _ => {
            if src.trim().is_empty() {
                return JsonDoc::default();
            }
            match parse_one(src, dialect == Dialect::Jsonc, 0, src) {
                Ok(n) => JsonDoc {
                    roots: vec![n],
                    error: None,
                },
                Err(e) => JsonDoc {
                    roots: Vec::new(),
                    error: Some(e),
                },
            }
        }
    }
}

/// The chain of nodes containing char `offset`, outermost first.
pub fn path_at(roots: &[JsonNode], offset: usize) -> Vec<&JsonNode> {
    let mut out = Vec::new();
    let mut level = roots;
    loop {
        let hit = level.iter().find(|n| {
            let start = n.key_range.map_or(n.range.0, |k| k.0);
            start <= offset && offset <= n.range.1
        });
        match hit {
            Some(n) => {
                out.push(n);
                level = &n.children;
            }
            None => return out,
        }
    }
}

/// `$.user.address.city` / `$.tags[2]` and `user.address.city` / `tags[2]`.
pub fn paths(chain: &[&JsonNode]) -> (String, String) {
    let mut json = String::from("$");
    let mut js = String::new();
    for (i, n) in chain.iter().enumerate().skip(1) {
        let parent = chain[i - 1];
        if parent.kind == Kind::Array {
            let idx = parent
                .children
                .iter()
                .position(|c| std::ptr::eq(c, *n))
                .unwrap_or(0);
            json.push_str(&format!("[{idx}]"));
            js.push_str(&format!("[{idx}]"));
        } else if let Some(k) = &n.key {
            let simple = !k.is_empty()
                && k.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                && !k.starts_with(|c: char| c.is_ascii_digit());
            if simple {
                json.push('.');
                json.push_str(k);
                if !js.is_empty() {
                    js.push('.');
                }
                js.push_str(k);
            } else {
                let q = serde_quote(k);
                json.push_str(&format!("[{q}]"));
                js.push_str(&format!("[{q}]"));
            }
        }
    }
    (json, js)
}

fn serde_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── formatting ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok<'a> {
    Open(u8),
    Close(u8),
    Colon,
    Comma,
    Atom(&'a str),
    /// Comment text and whether it followed a token on the same line.
    LineComment(&'a str, bool),
    BlockComment(&'a str, bool),
}

/// Tokenizes already-validated JSON/JSONC.
fn tokens(src: &str) -> Vec<Tok<'_>> {
    let s = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut newline_since_token = true;
    while i < s.len() {
        let c = s[i];
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                newline_since_token = true;
                i += 1;
            }
            b'{' | b'[' => {
                out.push(Tok::Open(c));
                i += 1;
                newline_since_token = false;
            }
            b'}' | b']' => {
                out.push(Tok::Close(c));
                i += 1;
                newline_since_token = false;
            }
            b':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            b',' => {
                out.push(Tok::Comma);
                i += 1;
                newline_since_token = false;
            }
            b'/' if s.get(i + 1) == Some(&b'/') => {
                let st = i;
                while i < s.len() && s[i] != b'\n' {
                    i += 1;
                }
                out.push(Tok::LineComment(
                    src[st..i].trim_end(),
                    !newline_since_token && !out.is_empty(),
                ));
            }
            b'/' if s.get(i + 1) == Some(&b'*') => {
                let st = i;
                i += 2;
                while i + 1 < s.len() && !(s[i] == b'*' && s[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(s.len());
                out.push(Tok::BlockComment(
                    &src[st..i],
                    !newline_since_token && !out.is_empty(),
                ));
            }
            b'"' => {
                let st = i;
                i += 1;
                while i < s.len() && s[i] != b'"' {
                    if s[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(s.len());
                out.push(Tok::Atom(&src[st..i]));
                newline_since_token = false;
            }
            _ => {
                let st = i;
                while i < s.len()
                    && !matches!(
                        s[i],
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b':' | b'}' | b']' | b'/'
                    )
                {
                    i += 1;
                }
                out.push(Tok::Atom(&src[st..i]));
                newline_since_token = false;
            }
        }
    }
    out
}

fn validate(src: &str, dialect: Dialect) -> Result<(), JsonError> {
    match parse(src, dialect).error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Format Document: pretty-prints with `indent`, keeping key order and
/// (JSONC) comments. Refuses invalid input. JSON Lines are formatted per line
/// without line breaks.
pub fn format(src: &str, dialect: Dialect, indent: &str) -> Result<String, JsonError> {
    validate(src, dialect)?;
    if dialect == Dialect::Lines {
        return minify(src, dialect);
    }
    let toks = tokens(src);
    let mut out = String::new();
    let mut level = 0usize;
    let mut pending_nl = false;
    let nl = |out: &mut String, level: usize| {
        out.push('\n');
        for _ in 0..level {
            out.push_str(indent);
        }
    };
    let next_significant = |i: usize| {
        toks[i + 1..]
            .iter()
            .find(|t| !matches!(t, Tok::LineComment(..) | Tok::BlockComment(..)))
    };
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        match t {
            Tok::LineComment(c, same) | Tok::BlockComment(c, same) => {
                if *same && !out.is_empty() {
                    out.push(' ');
                    out.push_str(c);
                } else {
                    if !out.is_empty() {
                        nl(&mut out, level);
                    }
                    out.push_str(c);
                }
                if matches!(t, Tok::LineComment(..)) || !*same {
                    pending_nl = true;
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        match t {
            Tok::Close(_) => {
                level = level.saturating_sub(1);
                nl(&mut out, level);
                pending_nl = false;
            }
            _ if pending_nl => {
                nl(&mut out, level);
                pending_nl = false;
            }
            _ => {}
        }
        match t {
            Tok::Open(c) => {
                out.push(*c as char);
                let close = if *c == b'{' { b'}' } else { b']' };
                if next_significant(i) == Some(&Tok::Close(close))
                    && !matches!(
                        toks.get(i + 1),
                        Some(Tok::LineComment(..) | Tok::BlockComment(..))
                    )
                {
                    out.push(close as char);
                    i += 2;
                    continue;
                }
                level += 1;
                pending_nl = true;
            }
            Tok::Close(c) => out.push(*c as char),
            Tok::Colon => out.push_str(": "),
            Tok::Comma => {
                // A trailing comma (JSONC) is dropped before the bracket.
                if !matches!(next_significant(i), Some(Tok::Close(_))) {
                    out.push(',');
                }
                pending_nl = true;
            }
            Tok::Atom(a) => out.push_str(a),
            Tok::LineComment(..) | Tok::BlockComment(..) => unreachable!(),
        }
        i += 1;
    }
    out.push('\n');
    Ok(out)
}

/// Minify: no whitespace; comments and trailing commas dropped.
pub fn minify(src: &str, dialect: Dialect) -> Result<String, JsonError> {
    validate(src, dialect)?;
    let one = |s: &str| {
        let toks = tokens(s);
        let mut out = String::new();
        for (i, t) in toks.iter().enumerate() {
            match t {
                Tok::Open(c) | Tok::Close(c) => out.push(*c as char),
                Tok::Colon => out.push(':'),
                Tok::Comma => {
                    let next = toks[i + 1..]
                        .iter()
                        .find(|t| !matches!(t, Tok::LineComment(..) | Tok::BlockComment(..)));
                    if !matches!(next, Some(Tok::Close(_))) {
                        out.push(',');
                    }
                }
                Tok::Atom(a) => out.push_str(a),
                Tok::LineComment(..) | Tok::BlockComment(..) => {}
            }
        }
        out
    };
    if dialect == Dialect::Lines {
        let lines: Vec<String> = src
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(one)
            .collect();
        return Ok(lines.join("\n") + "\n");
    }
    Ok(one(src))
}

/// Whether the source has comments (Sort Keys would drop them).
pub fn has_comments(src: &str) -> bool {
    tokens(src)
        .iter()
        .any(|t| matches!(t, Tok::LineComment(..) | Tok::BlockComment(..)))
}

/// Sort Keys: objects' keys sorted recursively, pretty-printed.
pub fn sort_keys(src: &str, dialect: Dialect, indent: &str) -> Result<String, JsonError> {
    let doc = parse(src, dialect);
    if let Some(e) = doc.error {
        return Err(e);
    }
    let chars: Vec<char> = src.chars().collect();
    fn write(n: &JsonNode, chars: &[char], out: &mut String, indent: &str, level: usize) {
        let pad = |out: &mut String, l: usize| {
            out.push('\n');
            for _ in 0..l {
                out.push_str(indent);
            }
        };
        match n.kind {
            Kind::Object | Kind::Array => {
                let (open, close) = if n.kind == Kind::Object {
                    ('{', '}')
                } else {
                    ('[', ']')
                };
                out.push(open);
                if n.children.is_empty() {
                    out.push(close);
                    return;
                }
                let mut kids: Vec<&JsonNode> = n.children.iter().collect();
                if n.kind == Kind::Object {
                    kids.sort_by(|a, b| a.key.cmp(&b.key));
                }
                for (i, c) in kids.iter().enumerate() {
                    pad(out, level + 1);
                    if let Some(kr) = c.key_range {
                        out.extend(&chars[kr.0..kr.1]);
                        out.push_str(": ");
                    }
                    write(c, chars, out, indent, level + 1);
                    if i + 1 < kids.len() {
                        out.push(',');
                    }
                }
                pad(out, level);
                out.push(close);
            }
            _ => out.extend(&chars[n.range.0..n.range.1]),
        }
    }
    let mut out = String::new();
    for (i, r) in doc.roots.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write(
            r,
            &chars,
            &mut out,
            if dialect == Dialect::Lines {
                ""
            } else {
                indent
            },
            0,
        );
    }
    if dialect == Dialect::Lines {
        // One value per line.
        return minify(
            &out.replace('\n', " ").replace("} {", "}\n{"),
            Dialect::Json,
        )
        .or(Ok(out));
    }
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(src: &str, r: SrcRange) -> String {
        src.chars().skip(r.0).take(r.1 - r.0).collect()
    }

    #[test]
    fn spans_and_values() {
        let src = r#"{"user":{"name":"Jöhn","age":42},"tags":[true,null,-1.5e3]}"#;
        let doc = parse(src, Dialect::Json);
        assert!(doc.error.is_none());
        let root = &doc.roots[0];
        let user = &root.children[0];
        assert_eq!(user.key.as_deref(), Some("user"));
        assert_eq!(slice(src, user.key_range.unwrap()), "\"user\"");
        assert_eq!(user.children[0].value.as_deref(), Some("Jöhn"));
        assert_eq!(slice(src, user.children[1].range), "42");
        let tags = &root.children[1];
        assert_eq!(tags.kind, Kind::Array);
        assert_eq!(tags.children[2].value.as_deref(), Some("-1.5e3"));
        let chain = path_at(&doc.roots, src.find("42").unwrap());
        assert_eq!(chain.last().unwrap().key.as_deref(), Some("age"));
        assert_eq!(paths(&chain), ("$.user.age".into(), "user.age".into()));
        let in_tags = path_at(&doc.roots, src.find("null").unwrap() + 1);
        assert_eq!(paths(&in_tags).0, "$.tags[1]");
    }

    #[test]
    fn errors_have_line_and_column() {
        let src = "{\n  \"a\": 1\n  \"b\": 2\n}";
        let e = parse(src, Dialect::Json).error.unwrap();
        assert_eq!((e.line, e.column), (3, 3));
        assert_eq!(e.message, "expected ',' or '}'");
        let e = parse("{\"a\": 1,}", Dialect::Json).error.unwrap();
        assert_eq!(e.message, "trailing comma not allowed");
        assert!(parse("[1, 2", Dialect::Json).error.is_some());
        assert!(parse("\"x", Dialect::Json).error.is_some());
    }

    // spec 25: "Comments allowed in tsconfig.json"
    #[test]
    fn jsonc_allows_comments_and_trailing_commas() {
        let src = "{\n  // compiler\n  \"strict\": true, /* yes */\n  \"lib\": [\"es2020\",],\n}\n";
        let doc = parse(src, Dialect::Jsonc);
        assert!(doc.error.is_none(), "{:?}", doc.error);
        assert_eq!(doc.roots[0].children.len(), 2);
        assert!(parse(src, Dialect::Json).error.is_some());
        assert_eq!(Dialect::for_language("jsonc"), Some(Dialect::Jsonc));
    }

    #[test]
    fn json_lines() {
        let src = "{\"a\":1}\n\n[2]\n{bad}\n";
        let doc = parse(src, Dialect::Lines);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(slice(src, doc.roots[1].range), "[2]");
        assert_eq!(doc.error.unwrap().line, 4);
    }

    #[test]
    fn format_keeps_order() {
        let out = format(r#"{"b":1,"a":[1,2],"e":{},"f":[]}"#, Dialect::Json, "  ").unwrap();
        assert_eq!(
            out,
            "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ],\n  \"e\": {},\n  \"f\": []\n}\n"
        );
        assert!(parse(&out, Dialect::Json).error.is_none());
    }

    #[test]
    fn format_keeps_comments() {
        let src = "{// top\n\"a\":1, // one\n/* block */\"b\":[1,],}";
        let out = format(src, Dialect::Jsonc, "  ").unwrap();
        assert_eq!(
            out,
            "{ // top\n  \"a\": 1, // one\n  /* block */\n  \"b\": [\n    1\n  ]\n}\n"
        );
        assert!(parse(&out, Dialect::Jsonc).error.is_none());
    }

    #[test]
    fn format_refuses_invalid() {
        let e = format("{\"a\" 1}", Dialect::Json, "  ").unwrap_err();
        assert_eq!((e.line, e.column), (1, 6));
    }

    #[test]
    fn minify_and_sort() {
        assert_eq!(
            minify("{ \"b\" : [1, 2,], // c\n \"a\": null }", Dialect::Jsonc).unwrap(),
            r#"{"b":[1,2],"a":null}"#
        );
        let sorted = sort_keys(
            r#"{"b":{"y":1,"x":2},"a":[{"d":1,"c":2}]}"#,
            Dialect::Json,
            "  ",
        )
        .unwrap();
        let doc = parse(&sorted, Dialect::Json);
        let keys: Vec<_> = doc.roots[0]
            .children
            .iter()
            .map(|c| c.key.clone().unwrap())
            .collect();
        assert_eq!(keys, ["a", "b"]);
        assert_eq!(
            doc.roots[0].children[1].children[0].key.as_deref(),
            Some("x")
        );
        assert_eq!(
            doc.roots[0].children[0].children[0].children[0]
                .key
                .as_deref(),
            Some("c")
        );
        assert!(has_comments("{} // x"));
    }
}
