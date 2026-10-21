//! Lossless YAML trees and edits. Source text remains authoritative; all
//! locations exposed to the editor are character offsets, never byte offsets.

use std::collections::HashSet;

use rowan::ast::AstNode;
use yaml_edit::{AsYaml, CoreScalarType, ScalarValue, SyntaxKind, YamlFile, YamlNode};

use crate::change::Edit;
use crate::json::{JsonDoc, JsonError, JsonNode, Kind};
use crate::json_edit::{self, NewValue, TreeEditError};

type SyntaxNode = rowan::SyntaxNode<yaml_edit::Lang>;

/// Shared tree operation; each language supplies its own source edit adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeOp {
    Value(String),
    Key(String),
    Add(NewValue),
    Delete,
    Move(isize),
}

fn syntax(node: &YamlNode) -> &SyntaxNode {
    node.as_node().expect("YAML AST nodes have syntax")
}

fn roots(file: &YamlFile) -> Vec<YamlNode> {
    file.documents()
        .filter_map(|doc| doc.syntax().children().find_map(YamlNode::from_syntax))
        .collect()
}

fn children(node: &YamlNode) -> Vec<YamlNode> {
    match node {
        YamlNode::Mapping(m) => m.entries().filter_map(|e| e.value_node()).collect(),
        YamlNode::Sequence(s) => s.values().collect(),
        _ => Vec::new(),
    }
}

fn at(file: &YamlFile, path: &[usize]) -> Option<YamlNode> {
    let (first, rest) = path.split_first()?;
    let mut node = roots(file).get(*first)?.clone();
    for &i in rest {
        node = children(&node).get(i)?.clone();
    }
    Some(node)
}

fn kind(s: &yaml_edit::Scalar) -> Kind {
    if s.is_quoted() || s.value().starts_with(['|', '>']) {
        return Kind::String;
    }
    if s.as_string().is_empty() {
        return Kind::Null;
    }
    match ScalarValue::classify_plain(&s.as_string()) {
        CoreScalarType::Null => Kind::Null,
        CoreScalarType::Boolean => Kind::Bool,
        CoreScalarType::Integer | CoreScalarType::Float => Kind::Number,
        CoreScalarType::String => Kind::String,
    }
}

fn key_text(key: &YamlNode) -> String {
    key.as_scalar()
        .map_or_else(|| key.to_string(), |s| s.as_string())
}

fn convert(node: &YamlNode, bytes: &[usize]) -> JsonNode {
    let range = |n: &SyntaxNode| {
        let r = n.text_range();
        (
            bytes.partition_point(|b| *b < u32::from(r.start()) as usize),
            bytes.partition_point(|b| *b < u32::from(r.end()) as usize),
        )
    };
    let (kind, value, children) = match node {
        YamlNode::Mapping(m) => (
            Kind::Object,
            None,
            m.entries()
                .filter_map(|entry| {
                    let key = entry.key_node()?;
                    let value = entry.value_node()?;
                    let mut child = convert(&value, bytes);
                    child.key = Some(key_text(&key));
                    child.key_range = Some(range(syntax(&key)));
                    Some(child)
                })
                .collect(),
        ),
        YamlNode::Sequence(s) => (
            Kind::Array,
            None,
            s.values().map(|n| convert(&n, bytes)).collect(),
        ),
        YamlNode::Scalar(s) => (
            kind(s),
            Some(if kind(s) == Kind::String {
                s.as_string()
            } else {
                s.value()
            }),
            Vec::new(),
        ),
        // References stay literal: expanding aliases can create cycles and
        // would make source selection and editing ambiguous.
        _ => (Kind::String, Some(node.to_string()), Vec::new()),
    };
    JsonNode {
        kind,
        key: None,
        key_range: None,
        range: range(syntax(node)),
        value,
        children,
    }
}

fn error(src: &str, byte: usize, message: String) -> JsonError {
    let byte = byte.min(src.len());
    let prefix: String = src
        .char_indices()
        .take_while(|(i, _)| *i < byte)
        .map(|(_, c)| c)
        .collect();
    JsonError {
        offset: prefix.chars().count(),
        line: prefix.chars().filter(|c| *c == '\n').count() + 1,
        column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        message,
    }
}

fn validate(file: &YamlFile, src: &str) -> Result<(), JsonError> {
    for doc in file.documents() {
        let mut anchors = HashSet::new();
        for token in doc
            .syntax()
            .descendants_with_tokens()
            .filter_map(|n| n.into_token())
        {
            match token.kind() {
                SyntaxKind::ANCHOR => {
                    anchors.insert(token.text().trim_start_matches('&').to_owned());
                }
                SyntaxKind::REFERENCE
                    if !anchors.contains(token.text().trim_start_matches('*')) =>
                {
                    return Err(error(
                        src,
                        u32::from(token.text_range().start()) as usize,
                        "Undefined YAML alias".into(),
                    ));
                }
                _ => {}
            }
        }
        for mapping in doc
            .syntax()
            .descendants()
            .filter_map(yaml_edit::Mapping::cast)
        {
            // Most YAML mappings use scalar keys. Hash their semantic value
            // so large flat configuration files do not require O(n²) scans.
            let mut scalar_keys = HashSet::new();
            let mut complex_keys: Vec<YamlNode> = Vec::new();
            for key in mapping.keys() {
                let duplicate = if let Some(scalar) = key.as_scalar() {
                    let value = scalar.as_string();
                    let identity = match kind(scalar) {
                        Kind::Null => "null:".to_owned(),
                        Kind::Bool => format!("bool:{}", value.to_ascii_lowercase()),
                        Kind::Number => {
                            if scalar
                                .syntax()
                                .first_token()
                                .is_some_and(|t| t.kind() == SyntaxKind::INT)
                            {
                                format!(
                                    "integer:{}",
                                    scalar.as_i64().map_or(value, |n| n.to_string())
                                )
                            } else {
                                format!(
                                    "float:{}",
                                    scalar.as_f64().map_or(value, |n| {
                                        if n == 0.0 { "0".into() } else { n.to_string() }
                                    })
                                )
                            }
                        }
                        _ => format!("string:{value}"),
                    };
                    !scalar_keys.insert(identity)
                } else {
                    let duplicate = complex_keys
                        .iter()
                        .any(|other| yaml_edit::yaml_eq(&key, other));
                    complex_keys.push(key.clone());
                    duplicate
                };
                if duplicate {
                    return Err(error(
                        src,
                        u32::from(syntax(&key).text_range().start()) as usize,
                        "Duplicate YAML key".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn file(src: &str) -> Result<YamlFile, JsonError> {
    let parsed = YamlFile::parse(src);
    if let Some(e) = parsed.positioned_errors().first() {
        return Err(error(src, e.range.start as usize, e.message.clone()));
    }
    let file = parsed.tree();
    validate(&file, src)?;
    Ok(file)
}

pub fn parse(src: &str) -> JsonDoc {
    match file(src) {
        Ok(file) => {
            let bytes: Vec<_> = src.char_indices().map(|(b, _)| b).collect();
            JsonDoc {
                roots: roots(&file).iter().map(|n| convert(n, &bytes)).collect(),
                error: None,
            }
        }
        Err(error) => JsonDoc {
            roots: Vec::new(),
            error: Some(error),
        },
    }
}

fn string_literal(input: &str, old: &str) -> String {
    if old.starts_with('\'') && !input.contains(['\n', '\r']) {
        format!("'{}'", input.replace('\'', "''"))
    } else if !old.starts_with(['"', '\'', '|', '>'])
        && !input.is_empty()
        && let Ok(f) = file(input)
        && let Some(YamlNode::Scalar(s)) = roots(&f).first()
        && kind(s) == Kind::String
        && s.as_string() == input
    {
        input.to_owned()
    } else {
        json_edit::quote(input)
    }
}

fn scalar_literal(scalar: &yaml_edit::Scalar, input: &str) -> Result<String, TreeEditError> {
    let old = scalar.value();
    match kind(scalar) {
        Kind::String => {
            // Block scalar payloads are replaced without consuming the next
            // sibling's indentation. Keep literal style when it can express
            // the requested text exactly; quoting is the safe fallback.
            let comment = old
                .lines()
                .next()
                .filter(|_| old.starts_with(['|', '>']))
                .and_then(|line| line.split_once('#'))
                .map_or(String::new(), |(_, comment)| format!(" #{comment}"));
            if old.starts_with('|')
                && !input.contains('\r')
                && input
                    .lines()
                    .find(|line| !line.is_empty())
                    .is_none_or(|line| !line.starts_with([' ', '\t']))
            {
                let indent = old
                    .lines()
                    .skip(1)
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.chars().take_while(|c| *c == ' ').count())
                    .unwrap_or(2);
                let trailing = input.chars().rev().take_while(|c| *c == '\n').count();
                let header = match trailing {
                    0 => "|-",
                    1 => "|",
                    _ => "|+",
                };
                let mut text = format!("{header}{comment}\n");
                for line in input.split_terminator('\n') {
                    text.push_str(&" ".repeat(indent));
                    text.push_str(line);
                    text.push('\n');
                }
                return Ok(text);
            }
            let mut text = string_literal(input, &old);
            text.push_str(&comment);
            if old.ends_with('\n') {
                text.push('\n');
            }
            Ok(text)
        }
        k @ (Kind::Number | Kind::Bool) => {
            let input = input.trim();
            let parsed = parse(input);
            if parsed.error.is_some() || parsed.roots.len() != 1 || parsed.roots[0].kind != k {
                return Err(TreeEditError::InvalidValue(if k == Kind::Number {
                    "number"
                } else {
                    "boolean"
                }));
            }
            Ok(input.to_owned())
        }
        _ => Err(TreeEditError::NotAllowed),
    }
}

/// Validate inline fields through the same adapter that executes them.
pub fn can_edit(src: &str, path: &[usize], op: &TreeOp) -> bool {
    edit(src, path, op).is_ok()
}

/// Mutate a fresh CST, validate the result, then return a localized source
/// replacement. Neither serializing through JSON nor saving a file is involved.
pub fn edit(
    src: &str,
    path: &[usize],
    op: &TreeOp,
) -> Result<(Vec<Edit>, Vec<usize>), TreeEditError> {
    let file = file(src).map_err(|_| TreeEditError::NotAllowed)?;
    let node = at(&file, path).ok_or(TreeEditError::NoNode)?;
    let mut next = path.to_vec();
    match op {
        TreeOp::Value(input) => {
            let scalar = node.as_scalar().ok_or(TreeEditError::NotAllowed)?;
            let range = scalar.syntax().text_range();
            let mut output = src.to_owned();
            output.replace_range(
                u32::from(range.start()) as usize..u32::from(range.end()) as usize,
                &scalar_literal(scalar, input)?,
            );
            let output = preserve_eol(src, &output);
            file_check(&output)?;
            return Ok((difference(src, &output), next));
        }
        TreeOp::Key(key) => {
            let parent = at(&file, &path[..path.len() - 1]).ok_or(TreeEditError::NotAllowed)?;
            let mapping = parent.as_mapping().ok_or(TreeEditError::NotAllowed)?;
            let entry = mapping
                .entries()
                .nth(*path.last().ok_or(TreeEditError::NoNode)?)
                .ok_or(TreeEditError::NoNode)?;
            let old_key = entry.key_node().ok_or(TreeEditError::NotAllowed)?;
            if mapping
                .keys()
                .any(|k| k != old_key && yaml_edit::yaml_eq(&k, &key.as_str()))
            {
                return Err(TreeEditError::DuplicateKey);
            }
            let scalar = old_key.as_scalar().ok_or(TreeEditError::NotAllowed)?;
            let range = scalar.syntax().text_range();
            let mut output = src.to_owned();
            output.replace_range(
                u32::from(range.start()) as usize..u32::from(range.end()) as usize,
                &string_literal(key, &scalar.value()),
            );
            file_check(&output)?;
            return Ok((difference(src, &output), next));
        }
        TreeOp::Add(value) => {
            let literal = match value {
                NewValue::String => "\"\"",
                NewValue::Number => "0",
                NewValue::Bool => "false",
                NewValue::Null => "null",
                NewValue::Object => "{}",
                NewValue::Array => "[]",
            };
            let value_file = file_literal(literal);
            let value = roots(&value_file).remove(0);
            next.push(children(&node).len());
            match &node {
                YamlNode::Mapping(m) => {
                    let mut key = "newKey".to_owned();
                    let mut n = 2;
                    while m.contains_key(key.as_str()) {
                        key = format!("newKey{n}");
                        n += 1;
                    }
                    m.set(key.as_str(), value);
                }
                YamlNode::Sequence(s) => s.push(value),
                _ => return Err(TreeEditError::NotAllowed),
            }
        }
        TreeOp::Delete | TreeOp::Move(_) => {
            let parent = at(&file, &path[..path.len() - 1]).ok_or(TreeEditError::NotAllowed)?;
            let i = *path.last().ok_or(TreeEditError::NoNode)?;
            if let TreeOp::Move(delta) = op {
                if !matches!(*delta, -1 | 1) {
                    return Err(TreeEditError::NotAllowed);
                }
                let j = i
                    .checked_add_signed(*delta)
                    .filter(|j| *j < children(&parent).len())
                    .ok_or(TreeEditError::NotAllowed)?;
                // Swap just the entry bodies: separators, indentation, and
                // between-entry comments remain in place, as in JSON edits.
                let entries: Vec<_> = syntax(&parent)
                    .children()
                    .filter(|n| {
                        matches!(
                            n.kind(),
                            SyntaxKind::MAPPING_ENTRY | SyntaxKind::SEQUENCE_ENTRY
                        )
                    })
                    .collect();
                let body = |n: &SyntaxNode| {
                    let start = u32::from(n.text_range().start()) as usize;
                    let end = n
                        .children()
                        .last()
                        .map_or(start, |c| u32::from(c.text_range().end()) as usize);
                    let text = &src[start..end];
                    text.strip_suffix("\r\n")
                        .or_else(|| text.strip_suffix('\n'))
                        .unwrap_or(text)
                        .to_owned()
                };
                let a = body(entries.get(i).ok_or(TreeEditError::NotAllowed)?);
                let b = body(entries.get(j).ok_or(TreeEditError::NotAllowed)?);
                let replace = |n: &SyntaxNode, old: &str, new: String| {
                    let start = u32::from(n.text_range().start()) as usize;
                    (start, start + old.len(), new)
                };
                let mut edits = vec![
                    replace(&entries[i], &a, b.clone()),
                    replace(&entries[j], &b, a),
                ];
                edits.sort_by_key(|e| e.0);
                let mut output = src.to_owned();
                for (a, b, text) in edits.into_iter().rev() {
                    output.replace_range(a..b, &text);
                }
                let actual = parse(&output);
                if actual.error.is_some() {
                    return Err(TreeEditError::NotAllowed);
                }
                let mut expected = parse(src).roots;
                let (root, parents) = path[..path.len() - 1]
                    .split_first()
                    .ok_or(TreeEditError::NotAllowed)?;
                let mut container = expected.get_mut(*root).ok_or(TreeEditError::NoNode)?;
                for &p in parents {
                    container = container.children.get_mut(p).ok_or(TreeEditError::NoNode)?;
                }
                container.children.swap(i, j);
                if !same_values(&expected, &actual.roots) {
                    return Err(TreeEditError::NotAllowed);
                }
                *next.last_mut().expect("node path") = j;
                return Ok((difference(src, &output), next));
            }
            if children(&parent).len() == 1 {
                let block = match &parent {
                    YamlNode::Mapping(m) => !m.is_flow_style(),
                    YamlNode::Sequence(s) => !s.is_flow_style(),
                    _ => false,
                };
                if block {
                    let entry = syntax(&parent)
                        .children()
                        .find(|n| {
                            matches!(
                                n.kind(),
                                SyntaxKind::MAPPING_ENTRY | SyntaxKind::SEQUENCE_ENTRY
                            )
                        })
                        .ok_or(TreeEditError::NoNode)?;
                    let range = entry.text_range();
                    let start = u32::from(range.start()) as usize;
                    let end = u32::from(range.end()) as usize;
                    let literal = if matches!(parent, YamlNode::Mapping(_)) {
                        "{}"
                    } else {
                        "[]"
                    };
                    let newline = if src[start..end].ends_with("\r\n") {
                        "\r\n"
                    } else if src[start..end].ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    };
                    let mut output = src.to_owned();
                    output.replace_range(start..end, &format!("{literal}{newline}"));
                    file_check(&output)?;
                    next.pop();
                    return Ok((difference(src, &output), next));
                }
            }
            match parent {
                YamlNode::Mapping(m) => m.entries().nth(i).ok_or(TreeEditError::NoNode)?.remove(),
                YamlNode::Sequence(s) => {
                    s.remove(i).ok_or(TreeEditError::NoNode)?;
                }
                _ => return Err(TreeEditError::NotAllowed),
            }
            if i == 0 {
                next.pop();
            } else {
                *next.last_mut().expect("node path") -= 1;
            }
        }
    }
    let output = preserve_eol(src, &file.to_string());
    file_check(&output)?;
    Ok((difference(src, &output), next))
}

fn file_literal(src: &str) -> YamlFile {
    YamlFile::parse(src).tree()
}
fn file_check(src: &str) -> Result<(), TreeEditError> {
    file(src).map(|_| ()).map_err(|_| TreeEditError::NotAllowed)
}
fn preserve_eol(src: &str, output: &str) -> String {
    if src.contains("\r\n") {
        output.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        output.to_owned()
    }
}

fn difference(src: &str, output: &str) -> Vec<Edit> {
    let a: Vec<_> = src.chars().collect();
    let b: Vec<_> = output.chars().collect();
    let start = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let suffix = a[start..]
        .iter()
        .rev()
        .zip(b[start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if start == a.len() && start == b.len() {
        return Vec::new();
    }
    vec![Edit::new(
        start,
        a.len() - suffix,
        b[start..b.len() - suffix].iter().collect::<String>(),
    )]
}

fn same_values(a: &[JsonNode], b: &[JsonNode]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.kind == b.kind
                && a.key == b.key
                && a.value == b.value
                && same_values(&a.children, &b.children)
        })
}

/// Normalize block indentation and separator spacing without rewriting scalar
/// contents, comments, tags, directives, or document order.
pub fn format(src: &str, indent: usize) -> Result<String, JsonError> {
    let file = file(src)?;
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut widths = vec![0usize];
    let tokens: Vec<_> = file
        .syntax()
        .descendants_with_tokens()
        .filter_map(|n| n.into_token())
        .collect();
    for (i, token) in tokens.iter().enumerate() {
        if token
            .parent_ancestors()
            .any(|n| n.kind() == SyntaxKind::SCALAR)
        {
            continue;
        }
        let start = u32::from(token.text_range().start()) as usize;
        let end = u32::from(token.text_range().end()) as usize;
        if token.kind() == SyntaxKind::INDENT && (start == 0 || src[..start].ends_with('\n')) {
            let width = token.text().len();
            while widths.len() > 1 && *widths.last().expect("base width") > width {
                widths.pop();
            }
            if *widths.last().expect("base width") < width {
                widths.push(width);
            }
            edits.push((start, end, " ".repeat((widths.len() - 1) * indent.max(1))));
        } else if token.kind() == SyntaxKind::WHITESPACE
            && i > 0
            && matches!(
                tokens[i - 1].kind(),
                SyntaxKind::COLON | SyntaxKind::COMMA | SyntaxKind::DASH
            )
            && !token.text().contains(['\r', '\n'])
        {
            edits.push((start, end, " ".into()));
        }
    }
    let mut output = src.to_owned();
    for (a, b, text) in edits.drain(..).rev() {
        output.replace_range(a..b, &text);
    }
    let output = preserve_eol(src, &output);
    // Never apply a formatting result that changes the parsed values.
    let before = parse(src);
    let after = parse(&output);
    fn values(nodes: &mut [JsonNode]) {
        for n in nodes {
            n.range = (0, 0);
            n.key_range = None;
            values(&mut n.children);
        }
    }
    let mut a = before.roots;
    let mut b = after.roots;
    values(&mut a);
    values(&mut b);
    if after.error.is_some() || a != b {
        return Err(error(src, 0, "Formatting would change YAML values".into()));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChangeSet;

    fn apply(src: &str, path: &[usize], op: TreeOp) -> String {
        let (edits, _) = edit(src, path, &op).unwrap();
        let mut rope = ropey::Rope::from_str(src);
        ChangeSet::new(edits).unwrap().apply(&mut rope);
        rope.to_string()
    }

    #[test]
    fn yaml_tree_keeps_unicode_ranges_types_and_document_roots() {
        let src = "name: 'José'\nflag: true\nitems: [1, null, yes]\n---\nother: 0x20\n";
        let parsed = parse(src);
        assert_eq!(parsed.error, None);
        assert_eq!(parsed.roots.len(), 2);
        let name = &parsed.roots[0].children[0];
        assert_eq!(name.value.as_deref(), Some("José"));
        assert_eq!(
            src.chars()
                .skip(name.range.0)
                .take(name.range.1 - name.range.0)
                .collect::<String>(),
            "'José'"
        );
        assert_eq!(parsed.roots[0].children[1].kind, Kind::Bool);
        assert_eq!(parsed.roots[0].children[2].children[2].kind, Kind::String);
        assert_eq!(parsed.roots[1].children[0].kind, Kind::Number);
    }

    #[test]
    fn yaml_scalar_and_key_edits_preserve_comments_quotes_and_crlf() {
        let src = "# settings\r\nname: 'José' # keep\r\nage: 42\r\n";
        let changed = apply(src, &[0, 0], TreeOp::Value("L'été".into()));
        assert_eq!(changed, src.replace("'José'", "'L''été'"));
        let changed = apply(&changed, &[0, 1], TreeOp::Key("years".into()));
        assert!(changed.ends_with("years: 42\r\n"));
        assert_eq!(
            edit(&changed, &[0, 1], &TreeOp::Key("name".into())),
            Err(TreeEditError::DuplicateKey)
        );
        assert!(edit(&changed, &[0, 1], &TreeOp::Value("not a number".into())).is_err());
    }

    #[test]
    fn yaml_add_delete_and_move_block_and_flow_collections() {
        for src in ["a: 1\nb: 2\n", "{a: 1, b: 2}"] {
            let moved = apply(src, &[0, 1], TreeOp::Move(-1));
            let parsed = parse(&moved);
            assert_eq!(parsed.error, None, "{moved}");
            assert_eq!(parsed.roots[0].children[0].key.as_deref(), Some("b"));
            let added = apply(&moved, &[0], TreeOp::Add(NewValue::String));
            assert_eq!(parse(&added).roots[0].children.len(), 3);
            let deleted = apply(&added, &[0, 2], TreeOp::Delete);
            assert_eq!(parse(&deleted).roots[0].children.len(), 2);
        }
        for src in ["- one\n- two\n", "[one, two]"] {
            let moved = apply(src, &[0, 1], TreeOp::Move(-1));
            assert_eq!(
                parse(&moved).roots[0].children[0].value.as_deref(),
                Some("two")
            );
            let added = apply(&moved, &[0], TreeOp::Add(NewValue::Object));
            assert_eq!(parse(&added).roots[0].children[2].kind, Kind::Object);
            let deleted = apply(&added, &[0, 2], TreeOp::Delete);
            assert_eq!(parse(&deleted).roots[0].children.len(), 2);
        }
    }

    #[test]
    fn yaml_multiline_aliases_tags_and_complex_keys_stay_literal() {
        let src = "base: &base {name: one}\ncopy: *base\ncustom: !thing hello\n? [a, b]\n: value\nmessage: |-\n  Hello\n  World\n";
        let parsed = parse(src);
        assert_eq!(parsed.error, None);
        assert_eq!(parsed.roots[0].children[1].value.as_deref(), Some("*base"));
        assert!(
            parsed.roots[0].children[2]
                .value
                .as_ref()
                .unwrap()
                .contains("!thing")
        );
        assert!(
            edit(src, &[0, 0], &TreeOp::Delete).is_err(),
            "must not leave dangling alias"
        );
        let changed = apply(src, &[0, 4], TreeOp::Value("New\nText".into()));
        assert_eq!(
            parse(&changed).roots[0].children[4].value.as_deref(),
            Some("New\nText")
        );
        assert!(changed.contains("!thing hello"));
    }

    #[test]
    fn yaml_invalid_input_has_character_locations() {
        let parsed = parse("é: [1, 2\n");
        let e = parsed.error.unwrap();
        assert!(e.line >= 1 && e.column >= 1);
        assert!(e.offset <= "é: [1, 2\n".chars().count());
        assert!(parse("a: 1\na: 2\n").error.is_some());
    }

    #[test]
    fn yaml_format_preserves_values_and_comments() {
        let src = "root:\n    name:    'José' # keep\n    nested:\n        value: 42\n";
        let output = format(src, 2).unwrap();
        assert_eq!(
            output,
            "root:\n  name: 'José' # keep\n  nested:\n    value: 42\n"
        );
        assert_eq!(format(&output, 2).unwrap(), output);
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::ChangeSet;

    fn changed(src: &str, path: &[usize], op: TreeOp) -> String {
        let (edits, _) =
            edit(src, path, &op).unwrap_or_else(|e| panic!("{src:?} {path:?} {op:?}: {e}"));
        let mut rope = ropey::Rope::from_str(src);
        ChangeSet::new(edits).unwrap().apply(&mut rope);
        rope.to_string()
    }

    #[test]
    fn empty_values_and_collections_remain_in_tree() {
        let parsed = parse("a:\nb: null\nc: []\nd: {}\n");
        assert_eq!(parsed.error, None);
        assert_eq!(parsed.roots[0].children.len(), 4);
        assert_eq!(parsed.roots[0].children[0].kind, Kind::Null);
        for (src, path, expected) in [
            ("a: {only: 1}\n", vec![0, 0, 0], Kind::Object),
            ("a:\n  only: 1\n", vec![0, 0, 0], Kind::Object),
            ("a:\n  - only\n", vec![0, 0, 0], Kind::Array),
        ] {
            let output = changed(src, &path, TreeOp::Delete);
            let parsed = parse(&output);
            assert_eq!(parsed.error, None);
            assert_eq!(parsed.roots[0].children[0].kind, expected, "{output}");
            assert!(parsed.roots[0].children[0].children.is_empty());
        }
    }

    #[test]
    fn add_all_types_to_nested_block_and_flow_collections() {
        for src in [
            "parent:\n    a: 1\n",
            "parent:\n    - one\n",
            "parent: {}\n",
            "parent: []\n",
        ] {
            for value in NewValue::ALL {
                let output = changed(src, &[0, 0], TreeOp::Add(value));
                let parsed = parse(&output);
                assert_eq!(parsed.error, None);
                let node = parsed.roots[0].children[0].children.last().unwrap();
                let expected = match value {
                    NewValue::String => Kind::String,
                    NewValue::Number => Kind::Number,
                    NewValue::Bool => Kind::Bool,
                    NewValue::Null => Kind::Null,
                    NewValue::Object => Kind::Object,
                    NewValue::Array => Kind::Array,
                };
                assert_eq!(node.kind, expected, "{output}");
            }
        }
    }

    #[test]
    fn block_scalar_edits_preserve_values_and_header_comments() {
        for src in [
            "message: | # keep\n  hello\nother: value\n",
            "message: >- # keep\n  hello\nother: value\n",
        ] {
            for input in [
                "  leading spaces\nnext",
                "one\ntwo\n\n",
                "",
                "\nleading blank\n",
            ] {
                let output = changed(src, &[0, 0], TreeOp::Value(input.into()));
                let parsed = parse(&output);
                assert_eq!(parsed.error, None, "{output}");
                assert_eq!(
                    parsed.roots[0].children[0].value.as_deref(),
                    Some(input),
                    "{output}"
                );
                assert!(output.contains("# keep"));
                assert_eq!(parsed.roots[0].children[1].value.as_deref(), Some("value"));
            }
        }
    }

    #[test]
    fn moving_block_scalars_keeps_trailing_newlines() {
        let src = "message: |+\n  hello\n\n\nother: value\n";
        let output = changed(src, &[0, 1], TreeOp::Move(-1));
        let original = parse(src);
        let moved = parse(&output);
        assert_eq!(
            moved.roots[0].children[1].value, original.roots[0].children[0].value,
            "{output}"
        );
    }

    #[test]
    fn moving_nested_entries_preserves_shape() {
        for (src, path) in [
            ("root:\n  a:\n    - one\n  b: two\n", vec![0, 0, 1]),
            (
                "- name: one\n  value: 1\n- name: two\n  value: 2\n",
                vec![0, 1],
            ),
        ] {
            let output = changed(src, &path, TreeOp::Move(-1));
            let parsed = parse(&output);
            assert_eq!(parsed.error, None, "{output}");
            assert_eq!(
                parsed.roots[0].children.len(),
                parse(src).roots[0].children.len(),
                "{output}"
            );
        }
    }

    #[test]
    fn formatting_nested_scalars_and_dedents_is_safe() {
        for src in [
            "a:\n    message: |\n        one\n        two\nb:\n    text: x\n",
            "a:\n    nested:\n        value: 1\nb:\n        value: 2\n",
            "a:\n    message: >-\n        one\n        two\n",
        ] {
            let output = format(src, 2).unwrap_or_else(|e| panic!("{src:?}: {e:?}"));
            assert_eq!(format(&output, 2).unwrap(), output);
        }
    }
}
