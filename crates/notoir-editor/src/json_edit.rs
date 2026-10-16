//! JSON tree edits (docs/specs/25 §Editing in the Tree): each operation on a
//! parsed document becomes the smallest text edits that do it, so the
//! surrounding formatting and comments stay as written and the result is one
//! ordinary undoable transaction.

use crate::change::Edit;
use crate::json::{JsonNode, Kind};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TreeEditError {
    #[error("no such node")]
    NoNode,
    #[error("not a valid {0} value")]
    InvalidValue(&'static str),
    #[error("the key already exists")]
    DuplicateKey,
    #[error("not possible here")]
    NotAllowed,
}

/// A value to add: its type decides the initial literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewValue {
    String,
    Number,
    Bool,
    Null,
    Object,
    Array,
}

impl NewValue {
    pub const ALL: [NewValue; 6] = [
        NewValue::String,
        NewValue::Number,
        NewValue::Bool,
        NewValue::Null,
        NewValue::Object,
        NewValue::Array,
    ];

    pub fn key(self) -> &'static str {
        match self {
            NewValue::String => "string",
            NewValue::Number => "number",
            NewValue::Bool => "boolean",
            NewValue::Null => "null",
            NewValue::Object => "object",
            NewValue::Array => "array",
        }
    }

    pub fn from_key(key: &str) -> Option<NewValue> {
        NewValue::ALL.into_iter().find(|v| v.key() == key)
    }

    fn literal(self) -> &'static str {
        match self {
            NewValue::String => "\"\"",
            NewValue::Number => "0",
            NewValue::Bool => "false",
            NewValue::Null => "null",
            NewValue::Object => "{}",
            NewValue::Array => "[]",
        }
    }
}

/// A JSON string literal for `s`.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// JSON number grammar (no leading `+`, no leading zeros, no `NaN`).
fn is_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !b.get(i).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !b.get(i).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    i == b.len()
}

fn node<'a>(roots: &'a [JsonNode], path: &[usize]) -> Option<&'a JsonNode> {
    let (first, rest) = path.split_first()?;
    let mut n = roots.get(*first)?;
    for i in rest {
        n = n.children.get(*i)?;
    }
    Some(n)
}

/// The node and its parent (`None` for roots).
fn with_parent<'a>(
    roots: &'a [JsonNode],
    path: &[usize],
) -> Result<(&'a JsonNode, Option<&'a JsonNode>), TreeEditError> {
    let n = node(roots, path).ok_or(TreeEditError::NoNode)?;
    let parent = match path.len() {
        0 => return Err(TreeEditError::NoNode),
        1 => None,
        len => Some(node(roots, &path[..len - 1]).ok_or(TreeEditError::NoNode)?),
    };
    Ok((n, parent))
}

/// Where a member starts: its key for object members.
fn start_of(n: &JsonNode) -> usize {
    n.key_range.map_or(n.range.0, |k| k.0.min(n.range.0))
}

fn chars(src: &str) -> Vec<char> {
    src.chars().collect()
}

/// The leading whitespace of the line containing char `at`.
fn line_indent(src: &[char], at: usize) -> String {
    let start = src[..at.min(src.len())]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |i| i + 1);
    src[start..]
        .iter()
        .take_while(|c| **c == ' ' || **c == '\t')
        .collect()
}

fn line_of(src: &[char], at: usize) -> usize {
    src[..at.min(src.len())]
        .iter()
        .filter(|c| **c == '\n')
        .count()
}

/// Replaces a scalar's value; `input` is the text typed for its type
/// (strings unquoted, numbers as written, `true` / `false`).
pub fn set_value(
    roots: &[JsonNode],
    path: &[usize],
    input: &str,
) -> Result<Vec<Edit>, TreeEditError> {
    let n = node(roots, path).ok_or(TreeEditError::NoNode)?;
    let literal = match n.kind {
        Kind::String => quote(input),
        Kind::Number => {
            let t = input.trim();
            if !is_number(t) {
                return Err(TreeEditError::InvalidValue("number"));
            }
            t.to_owned()
        }
        Kind::Bool => match input.trim() {
            t @ ("true" | "false") => t.to_owned(),
            _ => return Err(TreeEditError::InvalidValue("boolean")),
        },
        Kind::Null | Kind::Object | Kind::Array => return Err(TreeEditError::NotAllowed),
    };
    Ok(vec![Edit::new(n.range.0, n.range.1, literal)])
}

/// Renames an object member.
pub fn rename_key(
    roots: &[JsonNode],
    path: &[usize],
    key: &str,
) -> Result<Vec<Edit>, TreeEditError> {
    let (n, parent) = with_parent(roots, path)?;
    let (Some(range), Some(parent)) = (n.key_range, parent) else {
        return Err(TreeEditError::NotAllowed);
    };
    if n.key.as_deref() == Some(key) {
        return Ok(Vec::new());
    }
    if parent
        .children
        .iter()
        .any(|c| c.key.as_deref() == Some(key))
    {
        return Err(TreeEditError::DuplicateKey);
    }
    Ok(vec![Edit::new(range.0, range.1, quote(key))])
}

/// A key not used yet in `obj` (`base`, `base2`, …).
pub fn unique_key(obj: &JsonNode, base: &str) -> String {
    let taken = |k: &str| obj.children.iter().any(|c| c.key.as_deref() == Some(k));
    std::iter::once(base.to_owned())
        .chain((2..).map(|n| format!("{base}{n}")))
        .find(|k| !taken(k))
        .expect("unbounded")
}

/// Appends a member to an object (`key` required) or an array (`key`
/// ignored), indented like its siblings (`unit` per level when it has
/// none). Returns the edits and the char offset of the new member's start
/// in the edited text.
pub fn add_child(
    src: &str,
    roots: &[JsonNode],
    path: &[usize],
    key: Option<&str>,
    value: NewValue,
    unit: &str,
) -> Result<(Vec<Edit>, usize), TreeEditError> {
    let container = node(roots, path).ok_or(TreeEditError::NoNode)?;
    let entry = match (container.kind, key) {
        (Kind::Object, Some(k)) => {
            if container
                .children
                .iter()
                .any(|c| c.key.as_deref() == Some(k))
            {
                return Err(TreeEditError::DuplicateKey);
            }
            format!("{}: {}", quote(k), value.literal())
        }
        (Kind::Array, _) => value.literal().to_owned(),
        _ => return Err(TreeEditError::NotAllowed),
    };
    let s = chars(src);
    match container.children.last() {
        Some(last) => {
            let first = &container.children[0];
            let at = last.range.1;
            // Siblings on their own lines: a new line with their indent.
            let text = if line_of(&s, start_of(first)) != line_of(&s, container.range.0) {
                format!(",\n{}{entry}", line_indent(&s, start_of(last)))
            } else {
                format!(", {entry}")
            };
            let start = at + text.chars().count() - entry.chars().count();
            Ok((vec![Edit::new(at, at, text)], start))
        }
        None => {
            let open = container.range.0 + 1;
            let close = container.range.1.saturating_sub(1);
            let inner: String = s[open..close].iter().collect();
            let indent = line_indent(&s, container.range.0);
            if inner.trim().is_empty() {
                let text = format!("\n{indent}{unit}{entry}\n{indent}");
                let start = open + 1 + indent.chars().count() + unit.chars().count();
                Ok((vec![Edit::new(open, close, text)], start))
            } else {
                // Only comments inside: keep them, add after.
                let text = format!("\n{indent}{unit}{entry}");
                let start = close + 1 + indent.chars().count() + unit.chars().count();
                Ok((vec![Edit::new(close, close, text)], start))
            }
        }
    }
}

/// Deletes a member with its separating comma.
pub fn delete(roots: &[JsonNode], path: &[usize]) -> Result<Vec<Edit>, TreeEditError> {
    let (n, parent) = with_parent(roots, path)?;
    let parent = parent.ok_or(TreeEditError::NotAllowed)?;
    let i = *path.last().expect("non-empty");
    let sibs = &parent.children;
    let edit = if let Some(next) = sibs.get(i + 1) {
        Edit::new(start_of(n), start_of(next), "")
    } else if i > 0 {
        Edit::new(sibs[i - 1].range.1, n.range.1, "")
    } else {
        // The only member: leave empty brackets.
        Edit::new(parent.range.0 + 1, parent.range.1 - 1, "")
    };
    Ok(vec![edit])
}

/// Swaps a member with its neighbour (`delta` −1 up, +1 down); the text
/// between them stays where it is. Returns the edits and the moved
/// member's new path.
pub fn move_child(
    src: &str,
    roots: &[JsonNode],
    path: &[usize],
    delta: isize,
) -> Result<(Vec<Edit>, Vec<usize>), TreeEditError> {
    let (_, parent) = with_parent(roots, path)?;
    let parent = parent.ok_or(TreeEditError::NotAllowed)?;
    let i = *path.last().expect("non-empty");
    let j = i
        .checked_add_signed(delta)
        .filter(|j| *j < parent.children.len() && delta.abs() == 1)
        .ok_or(TreeEditError::NotAllowed)?;
    let s = chars(src);
    let span = |n: &JsonNode| (start_of(n), n.range.1);
    let text = |(a, b): (usize, usize)| -> String { s[a..b].iter().collect() };
    let (a, b) = (span(&parent.children[i]), span(&parent.children[j]));
    let mut edits = vec![Edit::new(a.0, a.1, text(b)), Edit::new(b.0, b.1, text(a))];
    edits.sort_by_key(|e| e.start);
    let mut moved = path.to_vec();
    *moved.last_mut().expect("non-empty") = j;
    Ok((edits, moved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{Dialect, parse};

    /// Applies edits (sorted, non-overlapping) to `src`.
    fn apply(src: &str, edits: &[Edit]) -> String {
        let s: Vec<char> = src.chars().collect();
        let mut out = String::new();
        let mut at = 0;
        for e in edits {
            out.extend(&s[at..e.start]);
            out.push_str(&e.text);
            at = e.end;
        }
        out.extend(&s[at..]);
        out
    }

    fn run(src: &str, f: impl Fn(&str, &[JsonNode]) -> Vec<Edit>) -> String {
        let doc = parse(src, Dialect::Jsonc);
        assert!(doc.error.is_none(), "{:?}", doc.error);
        let out = apply(src, &f(src, &doc.roots));
        assert!(parse(&out, Dialect::Jsonc).error.is_none(), "valid: {out}");
        out
    }

    const SRC: &str = "{\n  // settings\n  \"name\": \"notoir\",\n  \"size\": 12,\n  \"on\": true,\n  \"tags\": [1, 2, 3],\n  \"empty\": {}\n}\n";

    #[test]
    fn values_and_keys() {
        let out = run(SRC, |_, r| set_value(r, &[0, 0], "say \"hi\"\n").unwrap());
        assert!(out.contains("\"name\": \"say \\\"hi\\\"\\n\","));
        let out = run(SRC, |_, r| set_value(r, &[0, 1], " 1.5e3 ").unwrap());
        assert!(out.contains("\"size\": 1.5e3,"));
        let doc = parse(SRC, Dialect::Jsonc);
        for bad in ["01", "1.", "+1", "NaN", "", "1e"] {
            assert_eq!(
                set_value(&doc.roots, &[0, 1], bad),
                Err(TreeEditError::InvalidValue("number")),
                "{bad}"
            );
        }
        assert!(set_value(&doc.roots, &[0, 2], "yes").is_err());
        let out = run(SRC, |_, r| set_value(r, &[0, 2], "false").unwrap());
        assert!(out.contains("\"on\": false,"));
        let out = run(SRC, |_, r| rename_key(r, &[0, 1], "fontSize").unwrap());
        assert!(out.contains("\"fontSize\": 12,"));
        assert_eq!(
            rename_key(&doc.roots, &[0, 1], "on"),
            Err(TreeEditError::DuplicateKey)
        );
        // The comment is untouched by everything above.
        assert!(out.contains("// settings"));
    }

    #[test]
    fn add_members() {
        let out = run(SRC, |s, r| {
            add_child(s, r, &[0], Some("theme"), NewValue::String, "  ")
                .unwrap()
                .0
        });
        assert!(
            out.ends_with("  \"empty\": {},\n  \"theme\": \"\"\n}\n"),
            "{out}"
        );
        // Inline arrays stay inline.
        let out = run(SRC, |s, r| {
            add_child(s, r, &[0, 3], None, NewValue::Number, "  ")
                .unwrap()
                .0
        });
        assert!(out.contains("\"tags\": [1, 2, 3, 0],"));
        // Empty containers open a line indented one level deeper.
        let (edits, start) = {
            let doc = parse(SRC, Dialect::Jsonc);
            add_child(SRC, &doc.roots, &[0, 4], Some("a"), NewValue::Object, "  ").unwrap()
        };
        let out = apply(SRC, &edits);
        assert!(
            out.contains("  \"empty\": {\n    \"a\": {}\n  }\n}"),
            "{out}"
        );
        assert!(out[out.char_indices().nth(start).unwrap().0..].starts_with("\"a\""));
        let doc = parse(SRC, Dialect::Jsonc);
        assert_eq!(
            add_child(SRC, &doc.roots, &[0], Some("size"), NewValue::Null, "  "),
            Err(TreeEditError::DuplicateKey)
        );
        assert_eq!(unique_key(&doc.roots[0], "size"), "size2");
    }

    #[test]
    fn delete_and_move() {
        let out = run(SRC, |_, r| delete(r, &[0, 1]).unwrap());
        assert!(
            out.contains("\"name\": \"notoir\",\n  \"on\": true,"),
            "{out}"
        );
        let out = run(SRC, |_, r| delete(r, &[0, 4]).unwrap());
        assert!(out.ends_with("\"tags\": [1, 2, 3]\n}\n"), "{out}");
        let out = run(SRC, |_, r| delete(r, &[0, 3, 1]).unwrap());
        assert!(out.contains("[1, 3]"));
        let out = run("[\n  {\"a\": 1}\n]", |_, r| delete(r, &[0, 0]).unwrap());
        assert_eq!(out, "[]");
        let doc = parse(SRC, Dialect::Jsonc);
        assert_eq!(delete(&doc.roots, &[0]), Err(TreeEditError::NotAllowed));

        let out = run(SRC, |s, r| move_child(s, r, &[0, 3, 0], 1).unwrap().0);
        assert!(out.contains("[2, 1, 3]"));
        let out = run(SRC, |s, r| move_child(s, r, &[0, 1], -1).unwrap().0);
        assert!(
            out.contains("// settings\n  \"size\": 12,\n  \"name\": \"notoir\",\n"),
            "{out}"
        );
        assert_eq!(
            move_child(SRC, &doc.roots, &[0, 0], -1).map(|r| r.1),
            Err(TreeEditError::NotAllowed)
        );
        assert_eq!(
            move_child(SRC, &doc.roots, &[0, 3, 1], 1).unwrap().1,
            vec![0, 3, 2]
        );
    }
}
