//! Guesses a file's indentation from its first lines
//! (`editor.detectIndentation`, docs/specs/11 §Indentation).

use crate::edit_ops::IndentStyle;

const SAMPLE_LINES: usize = 1_000;

/// `None` when the sample has no indented lines.
pub fn detect(text: &str) -> Option<IndentStyle> {
    let (mut tab_lines, mut space_lines) = (0usize, 0usize);
    // Histogram of indentation increases between consecutive space-indented lines.
    let mut deltas = [0usize; 9];
    let mut prev = 0usize;
    for line in text.lines().take(SAMPLE_LINES) {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tab_lines += 1;
            prev = 0;
            continue;
        }
        let spaces = line.chars().take_while(|c| *c == ' ').count();
        if spaces > 0 {
            space_lines += 1;
        }
        if spaces > prev && spaces - prev <= 8 {
            deltas[spaces - prev] += 1;
        }
        prev = spaces;
    }
    if tab_lines == 0 && space_lines == 0 {
        return None;
    }
    if tab_lines > space_lines {
        return Some(IndentStyle {
            tab_size: 4,
            insert_spaces: false,
        });
    }
    // Prefer the most common step; ties go to the larger of 2/4/8.
    let size = [2usize, 4, 8, 3, 6, 1, 5, 7]
        .into_iter()
        .max_by_key(|&d| (deltas[d], d == 2 || d == 4 || d == 8, d))
        .filter(|&d| deltas[d] > 0)
        .unwrap_or(4);
    Some(IndentStyle {
        tab_size: size,
        insert_spaces: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_spaces_and_tabs() {
        let two = "a:\n  b:\n    c: 1\n  d: 2\n";
        assert_eq!(
            detect(two),
            Some(IndentStyle {
                tab_size: 2,
                insert_spaces: true
            })
        );
        let four = "fn x() {\n    if y {\n        z();\n    }\n}\n";
        assert_eq!(
            detect(four),
            Some(IndentStyle {
                tab_size: 4,
                insert_spaces: true
            })
        );
        let tabs = "fn x() {\n\tif y {\n\t\tz();\n\t}\n}\n";
        assert_eq!(detect(tabs).map(|s| s.insert_spaces), Some(false));
        assert_eq!(detect("no\nindent\n"), None);
    }
}
