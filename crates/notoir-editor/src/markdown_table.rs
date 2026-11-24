//! Lossless source-backed GFM table operations. All positions are characters.
use crate::{
    ChangeSet, Edit, Rope,
    markdown::{self, Align, Block, BlockKind, SrcRange},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub range: SrcRange,
    pub markdown: String,
    pub missing: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub range: SrcRange,
    pub rows: Vec<Vec<Cell>>,
    pub aligns: Vec<Align>,
    /// Prefix on continuation lines (blockquote/list indentation).
    pub prefix: String,
    pub trailing_newline: bool,
}

fn blocks<'a>(items: &'a [Block], out: &mut Vec<&'a Block>) {
    for b in items {
        match &b.kind {
            BlockKind::Table { .. } => out.push(b),
            BlockKind::Quote(children)
            | BlockKind::Footnote {
                blocks: children, ..
            } => blocks(children, out),
            BlockKind::List { items, .. } => {
                for item in items {
                    blocks(&item.blocks, out);
                }
            }
            _ => {}
        }
    }
}

pub fn tables(source: &str) -> Vec<Table> {
    let doc = markdown::parse(source);
    let mut found = Vec::new();
    blocks(&doc.blocks, &mut found);
    let rope = Rope::from_str(source);
    found
        .into_iter()
        .filter_map(|b| {
            let BlockKind::Table { aligns, .. } = &b.kind else {
                return None;
            };
            let raw = rope.slice(b.range.0..b.range.1).to_string();
            let mut rows = Vec::new();
            let mut at = b.range.0;
            let mut prefix = String::new();
            for (i, line) in raw.split_inclusive('\n').enumerate() {
                let body = line.trim_end_matches(['\r', '\n']);
                // The first line starts at the parser's table offset. Later lines
                // retain their enclosing container markers.
                let start = if i == 0 {
                    0
                } else if i == 1 {
                    body.chars()
                        .take_while(|c| c.is_whitespace() || *c == '>')
                        .count()
                } else if body.starts_with(&prefix) {
                    prefix.chars().count()
                } else {
                    0
                };
                if i == 1 {
                    prefix = body.chars().take(start).collect();
                }
                if i != 1 {
                    let chars: Vec<char> = body.chars().collect();
                    let mut cuts = Vec::new();
                    let mut backslashes = 0;
                    for (j, &c) in chars.iter().enumerate().skip(start) {
                        if c == '|' && backslashes % 2 == 0 {
                            cuts.push(j);
                        }
                        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
                    }
                    let first = chars[start..]
                        .iter()
                        .position(|c| !c.is_whitespace())
                        .map_or(chars.len(), |n| start + n);
                    let last = chars.iter().rposition(|c| !c.is_whitespace());
                    let mut boundaries = vec![start];
                    boundaries.extend(cuts.iter().map(|n| n + 1));
                    let mut cells = Vec::new();
                    for (n, &a) in boundaries.iter().enumerate() {
                        let z = cuts.get(n).copied().unwrap_or(chars.len());
                        if (n == 0 && cuts.first() == Some(&first))
                            || (n == cuts.len()
                                && last.is_some_and(|l| chars[l] == '|' && cuts.last() == Some(&l)))
                        {
                            continue;
                        }
                        let left = chars[a..z].iter().take_while(|c| c.is_whitespace()).count();
                        let right = chars[a..z]
                            .iter()
                            .rev()
                            .take_while(|c| c.is_whitespace())
                            .count();
                        let a = a + left;
                        let z = z.saturating_sub(right).max(a);
                        cells.push(Cell {
                            range: (at + a, at + z),
                            markdown: chars[a..z].iter().collect(),
                            missing: false,
                        });
                    }
                    // Missing cells have an insertion point before the final pipe.
                    while cells.len() < aligns.len() {
                        let end = at + last.filter(|l| chars[*l] == '|').unwrap_or(chars.len());
                        cells.push(Cell {
                            range: (end, end),
                            markdown: String::new(),
                            missing: true,
                        });
                    }
                    rows.push(cells);
                }
                at += line.chars().count();
            }
            (!rows.is_empty()).then(|| Table {
                range: b.range,
                rows,
                aligns: aligns.clone(),
                prefix,
                trailing_newline: raw.ends_with('\n'),
            })
        })
        .collect()
}

/// Preserve existing escapes; escape unescaped pipes, flatten physical breaks.
pub fn safe_cell(value: &str) -> String {
    let mut out = String::new();
    let mut slashes = 0;
    for c in value
        .replace("\r\n", " ")
        .replace(['\r', '\n', '\t'], " ")
        .chars()
    {
        if c == '|' && slashes % 2 == 0 {
            out.push('\\');
        }
        out.push(c);
        slashes = if c == '\\' { slashes + 1 } else { 0 };
    }
    // A trailing escape must not consume a compact row's next delimiter.
    if slashes % 2 == 1 {
        out.push('\\');
    }
    out
}

impl Table {
    /// Move data rows as one block, preserving their source order. `before` is
    /// a gap in the original table; the header cannot move or be displaced.
    pub fn move_rows(&self, rows: &[usize], before: usize) -> Option<(ChangeSet, Vec<usize>)> {
        let selected: std::collections::BTreeSet<_> = rows.iter().copied().collect();
        if selected.is_empty()
            || selected.contains(&0)
            || selected.last().copied()? >= self.rows.len()
            || !(1..=self.rows.len()).contains(&before)
        {
            return None;
        }
        let original = self.values();
        let moving: Vec<_> = selected.iter().map(|&r| original[r].clone()).collect();
        let mut values: Vec<_> = original
            .iter()
            .enumerate()
            .filter(|(r, _)| !selected.contains(r))
            .map(|(_, row)| row.clone())
            .collect();
        let at = before - selected.iter().filter(|&&r| r < before).count();
        values.splice(at..at, moving);
        if values == original {
            return None;
        }
        let moved = (at..at + selected.len()).collect();
        Some((self.replace(&values, &self.aligns)?, moved))
    }

    pub fn values(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|row| row.iter().map(|c| c.markdown.clone()).collect())
            .collect()
    }

    pub fn cell_edit(&self, row: usize, column: usize, value: &str) -> Option<ChangeSet> {
        let cell = self.rows.get(row)?.get(column)?;
        // For ragged input, materialize missing delimiters using the serializer.
        if self.rows[row].iter().any(|c| c.missing) {
            let mut values = self.values();
            values[row][column] = safe_cell(value);
            return self.replace(&values, &self.aligns);
        }
        Some(ChangeSet::replace(
            cell.range.0,
            cell.range.1,
            safe_cell(value),
        ))
    }

    pub fn replace(&self, values: &[Vec<String>], aligns: &[Align]) -> Option<ChangeSet> {
        if values.is_empty() || aligns.is_empty() {
            return None;
        }
        let mut text = String::new();
        let mut append = |cells: Vec<String>| {
            if !text.is_empty() {
                text.push('\n');
                text.push_str(&self.prefix);
            }
            text.push_str("| ");
            text.push_str(&cells.join(" | "));
            text.push_str(" |");
        };
        append(values[0].iter().map(|s| safe_cell(s)).collect());
        append(
            aligns
                .iter()
                .map(|a| {
                    match a {
                        Align::None => "---",
                        Align::Left => ":---",
                        Align::Center => ":---:",
                        Align::Right => "---:",
                    }
                    .into()
                })
                .collect(),
        );
        for row in &values[1..] {
            append(row.iter().map(|s| safe_cell(s)).collect());
        }
        if self.trailing_newline {
            text.push('\n');
        }
        // Validate the generated fragment independently of its container.
        let plain = text.replace(&format!("\n{}", self.prefix), "\n");
        if !matches!(
            markdown::parse(&plain).blocks.first().map(|b| &b.kind),
            Some(BlockKind::Table { .. })
        ) {
            return None;
        }
        ChangeSet::new(vec![Edit::new(self.range.0, self.range.1, text)]).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_nonadjacent_rows_preserves_order_alignment_and_surplus_cells() {
        let source = "outside\n\n| H | N |\n|:---|---:|\n| **α** | 1 | hidden |\n| b | 2 |\n| [c](target.md) | 3 |\n| d | 4 |\n\nafter\n";
        let table = tables(source).remove(0);
        let (edit, selected) = table.move_rows(&[1, 3], 5).unwrap();
        assert_eq!(selected, vec![3, 4]);
        let mut rope = Rope::from_str(source);
        edit.apply(&mut rope);
        let result = tables(&rope.to_string()).remove(0);
        assert_eq!(
            result
                .rows
                .iter()
                .map(|r| r[0].markdown.as_str())
                .collect::<Vec<_>>(),
            vec!["H", "b", "d", "**α**", "[c](target.md)"]
        );
        assert_eq!(result.aligns, table.aligns);
        assert_eq!(result.rows[3][2].markdown, "hidden");
        assert!(rope.to_string().starts_with("outside\n\n"));
        assert!(rope.to_string().ends_with("\nafter\n"));
    }
    #[test]
    fn row_moves_protect_the_header_and_ignore_unchanged_drops() {
        let table = tables("| H |\n|---|\n| a |\n| b |\n| c |\n").remove(0);
        for rows in [vec![], vec![0], vec![1, 0], vec![4]] {
            assert!(table.move_rows(&rows, 1).is_none());
        }
        assert!(table.move_rows(&[1], 0).is_none());
        assert!(table.move_rows(&[1], 5).is_none());
        for gap in 1..=3 {
            assert!(table.move_rows(&[1, 2], gap).is_none());
        }
        let (edit, selected) = table.move_rows(&[3], 1).unwrap();
        let mut rope = Rope::from_str("| H |\n|---|\n| a |\n| b |\n| c |\n");
        edit.apply(&mut rope);
        assert_eq!(selected, vec![1]);
        assert_eq!(tables(&rope.to_string())[0].rows[1][0].markdown, "c");
    }
    #[test]
    fn lossless_cell_edit_with_unicode_and_escaped_pipes() {
        let src = "é intro\n\n| **α** | `a\\|b` |\n|:---|---:|\n| x | y |\n\nafter\n";
        let table = tables(src).remove(0);
        assert_eq!(table.rows[0][1].markdown, "`a\\|b`");
        let mut rope = Rope::from_str(src);
        table.cell_edit(1, 0, "new|cell").unwrap().apply(&mut rope);
        assert_eq!(rope.to_string(), src.replace(" x ", " new\\|cell "));
    }
    #[test]
    fn nested_table_structure_keeps_container() {
        for src in [
            "> | a | b |\n> |---|---|\n> | x | y |\n",
            "- item\n\n  | a | b |\n  |---|---|\n  | x | y |\n",
        ] {
            let table = tables(src).remove(0);
            let mut values = table.values();
            values.push(vec!["z".into(), "q".into()]);
            let mut rope = Rope::from_str(src);
            table
                .replace(&values, &table.aligns)
                .unwrap()
                .apply(&mut rope);
            let parsed = tables(&rope.to_string());
            assert_eq!(parsed.len(), 1, "{rope}");
            assert_eq!(parsed[0].rows.len(), 3, "{rope}");
        }
    }
    #[test]
    fn header_only_and_empty_cells() {
        let t = tables("| | b |\n|---|---|\n").remove(0);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0][0].markdown, "");
        assert_eq!(safe_cell("a\r\nb\tc|d"), "a b c\\|d");
    }

    #[test]
    fn ragged_rows_materialize_missing_cells_without_merging_columns() {
        for src in [
            "a | b\n---|---\nx\n",
            "| a | b | c |\n|---|---|---|\n| x |\n",
        ] {
            let table = tables(src).remove(0);
            let last = table.aligns.len() - 1;
            let mut rope = Rope::from_str(src);
            table.cell_edit(1, last, "new").unwrap().apply(&mut rope);
            let parsed = tables(&rope.to_string()).remove(0);
            assert_eq!(parsed.rows[1][0].markdown, "x");
            assert_eq!(parsed.rows[1][last].markdown, "new");
        }
    }

    #[test]
    fn plain_table_cell_starting_with_angle_bracket_is_preserved() {
        let table = tables("| a | b |\n|---|---|\n| > text | value |\n").remove(0);
        assert_eq!(table.rows[1][0].markdown, "> text");
    }
}
