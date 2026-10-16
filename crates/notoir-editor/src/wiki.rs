//! Wiki links, tags and mentions in Markdown text (docs/specs/34): `[[Target]]`,
//! `[[Target#Heading|alias]]`, `#tag` in prose and front matter `tags:`.
//! Code (fenced blocks, inline spans) and front matter are skipped for links;
//! headings are skipped for tags. Ranges are char offsets.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikiLink {
    /// The whole `[[…]]`.
    pub range: (usize, usize),
    /// The target as written (`Architecture Decisions`, `notes/plan`).
    pub target: String,
    /// `#Heading` part.
    pub heading: Option<String>,
    /// `|alias` part.
    pub alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// Without `#`, as written.
    pub name: String,
    /// The `#tag` in prose; `None` for front matter tags.
    pub range: Option<(usize, usize)>,
}

/// One line of the document with what scanning may look at.
struct Line<'a> {
    /// Char offset of the line start.
    start: usize,
    text: &'a str,
    /// Inside a fenced code block or front matter.
    skipped: bool,
    heading: bool,
}

/// Splits `text` into lines, marking fenced code and front matter.
fn lines(text: &str) -> (Vec<Line<'_>>, Option<(usize, usize)>) {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut fence: Option<(char, usize)> = None;
    let mut front: Option<(usize, usize)> = None;
    let mut in_front = false;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim_start();
        let mut skipped = fence.is_some() || in_front;
        if i == 0 && line == "---" {
            in_front = true;
            skipped = true;
        } else if in_front && (line == "---" || line == "...") {
            in_front = false;
            skipped = true;
            front = Some((0, i));
        } else if !in_front {
            let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
            if let Some(m) = marker {
                let n = trimmed.chars().take_while(|c| *c == m).count();
                if n >= 3 {
                    match fence {
                        None => fence = Some((m, n)),
                        Some((fm, fnum)) if fm == m && n >= fnum => fence = None,
                        _ => {}
                    }
                    skipped = true;
                }
            }
        }
        let heading = !skipped && {
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            (1..=6).contains(&hashes) && trimmed[hashes..].starts_with([' ', '\t'])
                || (1..=6).contains(&hashes) && trimmed.len() == hashes
        };
        out.push(Line {
            start,
            text: line,
            skipped,
            heading,
        });
        start += raw.chars().count();
    }
    if in_front {
        // Unclosed front matter is ordinary text.
        for l in &mut out {
            l.skipped = false;
        }
    }
    (out, front)
}

/// Char ranges (within `line`) of inline code spans.
fn code_spans(line: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let n = chars[i..].iter().take_while(|c| **c == '`').count();
            let open = i;
            let mut j = i + n;
            let mut close = None;
            while j < chars.len() {
                if chars[j] == '`' {
                    let m = chars[j..].iter().take_while(|c| **c == '`').count();
                    if m == n {
                        close = Some(j + m);
                        break;
                    }
                    j += m;
                } else {
                    j += 1;
                }
            }
            match close {
                Some(end) => {
                    spans.push((open, end));
                    i = end;
                }
                None => i += n,
            }
        } else {
            i += 1;
        }
    }
    spans
}

fn in_spans(spans: &[(usize, usize)], at: usize) -> bool {
    spans.iter().any(|(a, b)| (*a..*b).contains(&at))
}

/// Every wiki link outside code and front matter.
pub fn links(text: &str) -> Vec<WikiLink> {
    let (lines, _) = lines(text);
    let mut out = Vec::new();
    for l in lines.iter().filter(|l| !l.skipped) {
        let chars: Vec<char> = l.text.chars().collect();
        let spans = code_spans(l.text);
        let mut i = 0;
        while i + 1 < chars.len() {
            if chars[i] == '[' && chars[i + 1] == '[' && !in_spans(&spans, i) {
                let body_start = i + 2;
                let close = (body_start..chars.len().saturating_sub(1))
                    .find(|&j| chars[j] == ']' && chars[j + 1] == ']');
                if let Some(j) = close {
                    let body: String = chars[body_start..j].iter().collect();
                    if !body.trim().is_empty() && !body.contains('[') {
                        let (main, alias) = match body.split_once('|') {
                            Some((m, a)) => (m, Some(a.trim().to_owned())),
                            None => (body.as_str(), None),
                        };
                        let (target, heading) = match main.split_once('#') {
                            Some((t, h)) => (t.trim(), Some(h.trim().to_owned())),
                            None => (main.trim(), None),
                        };
                        out.push(WikiLink {
                            range: (l.start + i, l.start + j + 2),
                            target: target.to_owned(),
                            heading: heading.filter(|h| !h.is_empty()),
                            alias: alias.filter(|a| !a.is_empty()),
                        });
                        i = j + 2;
                        continue;
                    }
                }
            }
            i += 1;
        }
    }
    out
}

/// The link containing char offset `at`.
pub fn link_at(text: &str, at: usize) -> Option<WikiLink> {
    links(text)
        .into_iter()
        .find(|l| (l.range.0..l.range.1).contains(&at))
}

fn tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

/// Tags in prose (not code, front matter or heading lines) and in front
/// matter `tags:` (`[a, b]`, `a, b` or a `- a` list).
pub fn tags(text: &str) -> Vec<Tag> {
    let (lines, front) = lines(text);
    let mut out = Vec::new();
    if let Some((from, to)) = front {
        let mut in_list = false;
        for l in &lines[from + 1..to] {
            let t = l.text.trim();
            if let Some(rest) = t.strip_prefix("tags:").or_else(|| t.strip_prefix("tag:")) {
                let rest = rest.trim().trim_start_matches('[').trim_end_matches(']');
                in_list = rest.is_empty();
                for name in rest.split(',') {
                    let name = name
                        .trim()
                        .trim_matches(['"', '\''])
                        .trim_start_matches('#');
                    if !name.is_empty() {
                        out.push(Tag {
                            name: name.to_owned(),
                            range: None,
                        });
                    }
                }
            } else if in_list && let Some(item) = t.strip_prefix("- ") {
                let name = item
                    .trim()
                    .trim_matches(['"', '\''])
                    .trim_start_matches('#');
                if !name.is_empty() {
                    out.push(Tag {
                        name: name.to_owned(),
                        range: None,
                    });
                }
            } else {
                in_list = false;
            }
        }
    }
    for l in lines.iter().filter(|l| !l.skipped && !l.heading) {
        let chars: Vec<char> = l.text.chars().collect();
        let spans = code_spans(l.text);
        let mut i = 0;
        while i < chars.len() {
            let boundary = i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(';
            if chars[i] == '#' && boundary && !in_spans(&spans, i) {
                let n = chars[i + 1..].iter().take_while(|c| tag_char(**c)).count();
                let name: String = chars[i + 1..i + 1 + n].iter().collect();
                let name = name.trim_end_matches(['/', '-']);
                if !name.is_empty() && name.chars().any(|c| !c.is_ascii_digit()) {
                    out.push(Tag {
                        name: name.to_owned(),
                        range: Some((l.start + i, l.start + i + 1 + name.chars().count())),
                    });
                }
                i += n + 1;
            } else {
                i += 1;
            }
        }
    }
    out
}

/// Whole-word, case-insensitive mentions of `name` outside wiki links, code
/// and front matter (unlinked mentions); (line, char range).
pub fn mentions(text: &str, name: &str) -> Vec<(usize, (usize, usize))> {
    let needle: Vec<char> = name.to_lowercase().chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let linked = links(text);
    let (lines, _) = lines(text);
    let mut out = Vec::new();
    for (n, l) in lines.iter().enumerate().filter(|(_, l)| !l.skipped) {
        let lower: Vec<char> = l.text.to_lowercase().chars().collect();
        let spans = code_spans(l.text);
        let mut i = 0;
        while i + needle.len() <= lower.len() {
            let before = i == 0 || !lower[i - 1].is_alphanumeric();
            let after = lower
                .get(i + needle.len())
                .is_none_or(|c| !c.is_alphanumeric());
            let at = l.start + i;
            if lower[i..i + needle.len()] == needle[..]
                && before
                && after
                && !in_spans(&spans, i)
                && !linked.iter().any(|w| (w.range.0..w.range.1).contains(&at))
            {
                out.push((n, (at, at + needle.len())));
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    out
}

/// After `[[`: the partial target typed before the caret (`text_before` is
/// the line up to the caret), unless the link is already closed.
pub fn completion_prefix(text_before: &str) -> Option<&str> {
    let open = text_before.rfind("[[")?;
    let partial = &text_before[open + 2..];
    if partial.contains("]]")
        || partial.contains('[')
        || partial.contains('|')
        || partial.contains('#')
    {
        return None;
    }
    Some(partial)
}

/// Rewrites links to `old` (a file stem, case-insensitive) as links to
/// `new`, keeping headings and aliases. Returns (char range, replacement).
pub fn retarget(text: &str, old: &str, new: &str) -> Vec<((usize, usize), String)> {
    links(text)
        .into_iter()
        .filter(|l| {
            let t = l.target.rsplit('/').next().unwrap_or(&l.target);
            let t = t.strip_suffix(".md").unwrap_or(t);
            t.eq_ignore_ascii_case(old) || t.to_lowercase() == old.to_lowercase()
        })
        .map(|l| {
            let mut s = format!("[[{new}");
            if let Some(h) = &l.heading {
                s.push('#');
                s.push_str(h);
            }
            if let Some(a) = &l.alias {
                s.push('|');
                s.push_str(a);
            }
            s.push_str("]]");
            (l.range, s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_with_headings_aliases_and_code() {
        let text = "We decided in [[Architecture Decisions]].\nSee [[Architecture Decisions#MCP|the MCP section]].\n`[[not a link]]` and\n```\n[[code]]\n```\n[[ ]] [[notes/plan]]\n";
        let l = links(text);
        let targets: Vec<&str> = l.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(
            targets,
            [
                "Architecture Decisions",
                "Architecture Decisions",
                "notes/plan"
            ]
        );
        assert_eq!(l[1].heading.as_deref(), Some("MCP"));
        assert_eq!(l[1].alias.as_deref(), Some("the MCP section"));
        let first = text.find("[[").unwrap();
        assert_eq!(
            l[0].range,
            (first, first + "[[Architecture Decisions]]".len())
        );
        assert_eq!(
            link_at(text, first + 3).unwrap().target,
            "Architecture Decisions"
        );
        assert!(link_at(text, 0).is_none());
    }

    #[test]
    fn front_matter_is_not_scanned_for_links_but_gives_tags() {
        let text = "---\ntitle: [[x]]\ntags: [rust, \"architecture\"]\naliases:\n---\n# Heading #nottag\nProse #meeting and #2024 and (#idea) a#b `#code`\n";
        assert!(links(text).is_empty());
        let names: Vec<String> = tags(text).into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["rust", "architecture", "meeting", "idea"]);
        let list = "---\ntags:\n  - one\n  - two\n---\nx\n";
        let names: Vec<String> = tags(list).into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["one", "two"]);
    }

    #[test]
    fn mentions_skip_links_and_code() {
        let text = "Project Plan is here.\n[[Project Plan]] linked.\n`Project Plan` code. project plan again; projectplans no.\n";
        let m = mentions(text, "Project Plan");
        assert_eq!(m.iter().map(|(l, _)| *l).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn completion_and_retarget() {
        assert_eq!(completion_prefix("see [[Arch"), Some("Arch"));
        assert_eq!(completion_prefix("see [["), Some(""));
        assert_eq!(completion_prefix("see [[Arch]] x"), None);
        assert_eq!(completion_prefix("see [[Arch#H"), None);
        assert_eq!(completion_prefix("no link"), None);
        let text = "a [[Old Name]] b [[old name#H|x]] c [[Other]]";
        let edits = retarget(text, "Old Name", "New Name");
        assert_eq!(
            edits.iter().map(|e| e.1.as_str()).collect::<Vec<_>>(),
            ["[[New Name]]", "[[New Name#H|x]]"]
        );
    }
}
