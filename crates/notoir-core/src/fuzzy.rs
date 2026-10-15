//! Fuzzy matching for filters and quick pickers: the query's characters must
//! appear in order (case-insensitive); consecutive runs, word starts and the
//! file name score higher.

/// A match: higher scores rank first; `positions` are char indices into the
/// candidate, for highlighting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub positions: Vec<usize>,
}

fn is_boundary(prev: Option<char>, c: char) -> bool {
    match prev {
        None => true,
        Some(p) => {
            matches!(p, '/' | '\\' | '_' | '-' | '.' | ' ')
                || (p.is_lowercase() && c.is_uppercase())
        }
    }
}

/// Matches `query` against `candidate`. Spaces in the query are ignored. An
/// empty query matches everything with score 0.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    let chars: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    // Where the last path component starts: matches there count more.
    let name_start = chars
        .iter()
        .rposition(|c| *c == '/' || *c == '\\')
        .map_or(0, |i| i + 1);

    // Prefer a match that starts as late as possible within the file name:
    // try every start position of the first query char from the name onwards,
    // then fall back to the leftmost greedy match.
    let mut best: Option<FuzzyMatch> = None;
    let starts: Vec<usize> = (0..lower.len()).filter(|&i| lower[i] == query[0]).collect();
    for &start in starts.iter().rev().take(16) {
        if let Some(m) = match_from(&query, &chars, &lower, start, name_start)
            && best.as_ref().is_none_or(|b| m.score > b.score)
        {
            best = Some(m);
        }
    }
    best
}

fn match_from(
    query: &[char],
    chars: &[char],
    lower: &[char],
    start: usize,
    name_start: usize,
) -> Option<FuzzyMatch> {
    let mut positions = Vec::with_capacity(query.len());
    let mut qi = 0;
    let mut i = start;
    while qi < query.len() && i < lower.len() {
        if lower[i] == query[qi] {
            positions.push(i);
            qi += 1;
        }
        i += 1;
    }
    if qi < query.len() {
        return None;
    }
    let mut score = 0;
    let mut prev_pos: Option<usize> = None;
    for &p in &positions {
        score += 1;
        match prev_pos {
            Some(q) if q + 1 == p => score += 8,
            // Gaps between matched characters cost a little.
            Some(q) => score -= ((p - q - 1) as i32).min(5),
            None => {}
        }
        if is_boundary(p.checked_sub(1).map(|i| chars[i]), chars[p]) {
            score += 8;
        }
        if p >= name_start {
            score += 3;
        }
        prev_pos = Some(p);
    }
    if positions[0] == name_start {
        score += 6;
    }
    // Shorter candidates and tighter matches win ties.
    let span = positions[positions.len() - 1] - positions[0];
    score -= (span as i32) / 4;
    score -= (chars.len() as i32) / 16;
    Some(FuzzyMatch { score, positions })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("rdm", "README.md").is_some());
        assert!(fuzzy_match("mdr", "README.md").is_none());
        assert_eq!(fuzzy_match("", "x").unwrap().score, 0);
    }

    #[test]
    fn positions_for_highlight() {
        let m = fuzzy_match("read", "docs/README.md").unwrap();
        assert_eq!(m.positions, vec![5, 6, 7, 8]);
    }

    #[test]
    fn consecutive_runs_beat_scattered_word_starts() {
        let a = fuzzy_match("tog", "View: Toggle Line Numbers")
            .unwrap()
            .score;
        let b = fuzzy_match("tog", "View: Move Tab to Next Group")
            .unwrap()
            .score;
        assert!(a > b, "{a} vs {b}");
    }

    #[test]
    fn file_name_and_word_starts_rank_higher() {
        let a = fuzzy_match("conf", "src/config.rs").unwrap().score;
        let b = fuzzy_match("conf", "cases/old/notes/f.rs").map_or(i32::MIN, |m| m.score);
        assert!(a > b);
        let c = fuzzy_match("ws", "workspace_state.rs").unwrap().score;
        let d = fuzzy_match("ws", "crews.rs").unwrap().score;
        assert!(c > d);
    }
}
