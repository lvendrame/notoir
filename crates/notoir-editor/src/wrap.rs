//! Column metrics and word wrapping for a monospace grid
//! (docs/specs/11 §Editor Widget Architecture, 14 §Word Wrap).
//!
//! Wrapping is computed here, not by the renderer, so display-row math and
//! painting always agree. Columns: tabs advance to the next tab stop,
//! East-Asian wide characters take two columns, combining marks zero.

use unicode_width::UnicodeWidthChar;

/// Columns occupied by `c` when it starts at column `col`.
pub fn char_cols(c: char, col: usize, tab: usize) -> usize {
    if c == '\t' {
        let tab = tab.max(1);
        tab - col % tab
    } else if c.is_control() {
        1
    } else {
        UnicodeWidthChar::width(c).unwrap_or(1)
    }
}

/// Visual column after the first `chars` characters of `line`.
pub fn col_of(line: &str, chars: usize, tab: usize) -> usize {
    let mut col = 0;
    for c in line.chars().take(chars) {
        col += char_cols(c, col, tab);
    }
    col
}

/// Char index in `line` whose start column is closest to `target` (clicks,
/// vertical motion). Never past the line's last char.
pub fn char_at_col(line: &str, target: usize, tab: usize) -> usize {
    let mut col = 0;
    for (i, c) in line.chars().enumerate() {
        let w = char_cols(c, col, tab);
        if col + w > target {
            // Round to the nearer edge of the character.
            return if (target - col) * 2 >= w { i + 1 } else { i };
        }
        col += w;
    }
    line.chars().count()
}

/// Char offsets (within `line`) where each display row starts; always begins
/// with `0`. Breaks after whitespace when possible, else mid-word; trailing
/// whitespace may hang past the edge.
pub fn wrap_line(line: &str, max_cols: usize, tab: usize) -> Vec<usize> {
    let max_cols = max_cols.max(1);
    let mut starts = vec![0];
    let mut row_start = 0usize;
    let mut col = 0usize;
    // Char index just after the last whitespace in the current row.
    let mut break_at: Option<usize> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let w = char_cols(c, col, tab);
        if col + w > max_cols && col > 0 && !c.is_whitespace() {
            let at = match break_at {
                Some(b) if b > row_start => b,
                _ => i,
            };
            starts.push(at);
            row_start = at;
            break_at = None;
            col = 0;
            i = at;
            continue;
        }
        col += w;
        if c.is_whitespace() {
            break_at = Some(i + 1);
        }
        i += 1;
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_with_tabs_and_wide_chars() {
        assert_eq!(col_of("a\tb", 2, 4), 4);
        assert_eq!(col_of("日本", 2, 4), 4);
        assert_eq!(col_of("e\u{301}x", 2, 4), 1, "combining mark is zero-width");
        assert_eq!(char_at_col("a\tb", 4, 4), 2);
        assert_eq!(char_at_col("abc", 99, 4), 3);
        assert_eq!(char_at_col("日本", 2, 4), 1);
    }

    #[test]
    fn wraps_at_word_boundaries() {
        assert_eq!(wrap_line("hello world foo", 11, 4), vec![0, 12]);
        assert_eq!(wrap_line("hello world", 5, 4), vec![0, 6]);
        assert_eq!(wrap_line("short", 80, 4), vec![0]);
        assert_eq!(wrap_line("", 10, 4), vec![0]);
    }

    #[test]
    fn long_words_break_mid_word() {
        assert_eq!(wrap_line("abcdefghij", 4, 4), vec![0, 4, 8]);
    }

    #[test]
    fn wide_chars_do_not_split_columns() {
        // Each CJK char is 2 columns; 5 columns fit two chars.
        assert_eq!(wrap_line("日本語テキスト", 5, 4), vec![0, 2, 4, 6]);
    }

    #[test]
    fn trailing_spaces_hang() {
        assert_eq!(wrap_line("abc     ", 4, 4), vec![0]);
    }
}
