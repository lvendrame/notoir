//! In-document find and replace (docs/specs/11 §Find / Replace): plain or
//! regex queries with case and whole-word options, optionally limited to a
//! range; matches are char ranges. Replace All is one change set; regex
//! replacements expand `$1` / `${name}`.

use regex::{Regex, RegexBuilder};
use ropey::Rope;

use crate::change::{ChangeSet, Edit};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Search only inside this char range (find in selection).
    pub within: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FindError {
    /// The regex does not compile; the message is shown inline.
    #[error("{0}")]
    InvalidRegex(String),
}

/// Escapes text for use as a regex (seeding the box in regex mode).
pub fn escape(text: &str) -> String {
    regex::escape(text)
}

/// Upper bound on reported matches (rendering and Replace All stay bounded).
pub const MAX_MATCHES: usize = 100_000;

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The compiled query.
pub struct Matcher {
    re: Regex,
    whole_word: bool,
}

impl Matcher {
    pub fn new(q: &FindQuery) -> Result<Option<Matcher>, FindError> {
        if q.text.is_empty() {
            return Ok(None);
        }
        let pattern = if q.regex {
            q.text.clone()
        } else {
            regex::escape(&q.text)
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!q.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| FindError::InvalidRegex(short_error(&e.to_string())))?;
        Ok(Some(Matcher {
            re,
            whole_word: q.whole_word,
        }))
    }

    /// Byte ranges of matches in `text`.
    fn byte_matches<'a>(&'a self, text: &'a str) -> impl Iterator<Item = regex::Match<'a>> + 'a {
        self.re.find_iter(text).filter(move |m| {
            if m.start() == m.end() {
                return false;
            }
            if !self.whole_word {
                return true;
            }
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            before.is_none_or(|c| !is_word_char(c)) && after.is_none_or(|c| !is_word_char(c))
        })
    }
}

/// The last line of a regex error is the useful part.
fn short_error(e: &str) -> String {
    e.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(e)
        .trim()
        .trim_start_matches("error: ")
        .to_owned()
}

/// All matches as char ranges, in order.
pub fn find_all(rope: &Rope, q: &FindQuery) -> Result<Vec<(usize, usize)>, FindError> {
    let Some(m) = Matcher::new(q)? else {
        return Ok(Vec::new());
    };
    let (from, to) = q.within.unwrap_or((0, rope.len_chars()));
    let to = to.min(rope.len_chars());
    let text = rope.slice(from..to).to_string();
    let base_byte = rope.char_to_byte(from);
    let mut out = Vec::new();
    for hit in m.byte_matches(&text).take(MAX_MATCHES) {
        let a = rope.byte_to_char(base_byte + hit.start());
        let b = rope.byte_to_char(base_byte + hit.end());
        out.push((a, b));
    }
    Ok(out)
}

/// The replacement for one match (regex captures expanded).
pub fn replacement(
    rope: &Rope,
    q: &FindQuery,
    m: (usize, usize),
    with: &str,
) -> Result<String, FindError> {
    if !q.regex {
        return Ok(with.to_owned());
    }
    let Some(matcher) = Matcher::new(q)? else {
        return Ok(with.to_owned());
    };
    let hay = rope.slice(m.0..m.1).to_string();
    // Re-run anchored on the match text so captures are available.
    match matcher.re.captures(&hay) {
        Some(caps) => {
            let mut out = String::new();
            caps.expand(with, &mut out);
            Ok(out)
        }
        None => Ok(with.to_owned()),
    }
}

/// Replace All as one change set; returns it with the replacement count.
pub fn replace_all(
    rope: &Rope,
    q: &FindQuery,
    with: &str,
) -> Result<Option<(ChangeSet, usize)>, FindError> {
    let Some(matcher) = Matcher::new(q)? else {
        return Ok(None);
    };
    let (from, to) = q.within.unwrap_or((0, rope.len_chars()));
    let to = to.min(rope.len_chars());
    let text = rope.slice(from..to).to_string();
    let base_byte = rope.char_to_byte(from);
    let mut edits = Vec::new();
    for caps in matcher.re.captures_iter(&text) {
        let m = caps.get(0).expect("group 0");
        if m.start() == m.end() {
            continue;
        }
        if matcher.whole_word {
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
                continue;
            }
        }
        let rep = if q.regex {
            let mut out = String::new();
            caps.expand(with, &mut out);
            out
        } else {
            with.to_owned()
        };
        let a = rope.byte_to_char(base_byte + m.start());
        let b = rope.byte_to_char(base_byte + m.end());
        edits.push(Edit::new(a, b, rep));
        if edits.len() >= MAX_MATCHES {
            break;
        }
    }
    let n = edits.len();
    if n == 0 {
        return Ok(None);
    }
    Ok(ChangeSet::new(edits).ok().map(|cs| (cs, n)))
}

/// Index of the first match at or after `pos` (wrapping); `true` when it
/// wrapped around.
pub fn next_from(matches: &[(usize, usize)], pos: usize, forward: bool) -> Option<(usize, bool)> {
    if matches.is_empty() {
        return None;
    }
    if forward {
        match matches.iter().position(|m| m.0 >= pos) {
            Some(i) => Some((i, false)),
            None => Some((0, true)),
        }
    } else {
        match matches.iter().rposition(|m| m.1 <= pos && m.0 < pos) {
            Some(i) => Some((i, false)),
            None => Some((matches.len() - 1, true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> FindQuery {
        FindQuery {
            text: text.into(),
            ..Default::default()
        }
    }

    // spec 11: "Whole word"
    #[test]
    fn whole_word() {
        let rope = Rope::from_str("cat concat cat");
        assert_eq!(find_all(&rope, &q("cat")).unwrap().len(), 3);
        let w = FindQuery {
            whole_word: true,
            ..q("cat")
        };
        assert_eq!(find_all(&rope, &w).unwrap(), vec![(0, 3), (11, 14)]);
    }

    // spec 11: "Regex replace with capture groups"
    #[test]
    fn regex_replace_with_captures() {
        let rope = Rope::from_str("2026-10-14");
        let r = FindQuery {
            regex: true,
            ..q(r"(\d+)-(\d+)-(\d+)")
        };
        let (cs, n) = replace_all(&rope, &r, "$3/$2/$1").unwrap().unwrap();
        let mut out = rope.clone();
        cs.apply(&mut out);
        assert_eq!(out.to_string(), "14/10/2026");
        assert_eq!(n, 1);
        let named = FindQuery {
            regex: true,
            ..q(r"(?P<y>\d{4})")
        };
        let m = find_all(&rope, &named).unwrap()[0];
        assert_eq!(replacement(&rope, &named, m, "<${y}>").unwrap(), "<2026>");
    }

    // spec 11: "Invalid regex"
    #[test]
    fn invalid_regex_is_an_error() {
        let rope = Rope::from_str("x");
        let r = FindQuery {
            regex: true,
            ..q("(")
        };
        assert!(matches!(
            find_all(&rope, &r),
            Err(FindError::InvalidRegex(_))
        ));
    }

    #[test]
    fn case_unicode_ranges_and_selection() {
        let rope = Rope::from_str("Ünïcode ünïcode\nÜNÏCODE");
        assert_eq!(
            find_all(&rope, &q("ünï")).unwrap(),
            vec![(0, 3), (8, 11), (16, 19)]
        );
        let cs = FindQuery {
            case_sensitive: true,
            ..q("ünï")
        };
        assert_eq!(find_all(&rope, &cs).unwrap(), vec![(8, 11)]);
        let within = FindQuery {
            within: Some((4, 15)),
            ..q("ünï")
        };
        assert_eq!(find_all(&rope, &within).unwrap(), vec![(8, 11)]);
        let multiline = FindQuery {
            regex: true,
            ..q(r"code\nÜ")
        };
        assert_eq!(find_all(&rope, &multiline).unwrap(), vec![(11, 17)]);
    }

    #[test]
    fn next_wraps() {
        let m = vec![(2, 3), (10, 11)];
        assert_eq!(next_from(&m, 0, true), Some((0, false)));
        assert_eq!(next_from(&m, 4, true), Some((1, false)));
        assert_eq!(next_from(&m, 12, true), Some((0, true)));
        assert_eq!(next_from(&m, 10, false), Some((0, false)));
        assert_eq!(next_from(&m, 1, false), Some((1, true)));
    }
}
