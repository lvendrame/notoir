//! Lossless delimited text: cells retain character ranges into the source.
use crate::ChangeSet;

#[derive(Clone, Debug)]
pub struct Cell {
    pub value: String,
    pub range: (usize, usize),
    pub quoted: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
    pub columns: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub column: usize,
}

pub fn parse(text: &str, separator: char) -> Result<Table, Error> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = usize::from(chars.first() == Some(&'\u{feff}'));
    let mut table = Table::default();
    while i < chars.len() {
        let mut row = Vec::new();
        loop {
            let start = i;
            let quoted = chars.get(i) == Some(&'"');
            let mut value = String::new();
            if quoted {
                i += 1;
                loop {
                    match chars.get(i) {
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            value.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(c) => {
                            value.push(*c);
                            i += 1;
                        }
                        None => return Err(location(&chars, i)),
                    }
                }
                if chars
                    .get(i)
                    .is_some_and(|c| *c != separator && *c != '\n' && *c != '\r')
                {
                    return Err(location(&chars, i));
                }
            } else {
                while let Some(&c) = chars.get(i) {
                    if c == separator || c == '\n' || c == '\r' {
                        break;
                    }
                    if c == '"' {
                        return Err(location(&chars, i));
                    }
                    value.push(c);
                    i += 1;
                }
            }
            row.push(Cell {
                value,
                range: (start, i),
                quoted,
            });
            if chars.get(i) == Some(&separator) {
                i += 1;
                continue;
            }
            if chars.get(i) == Some(&'\r') {
                i += 1;
            }
            if chars.get(i) == Some(&'\n') {
                i += 1;
            }
            break;
        }
        table.columns = table.columns.max(row.len());
        table.rows.push(row);
    }
    Ok(table)
}
fn location(chars: &[char], at: usize) -> Error {
    let before = &chars[..at];
    Error {
        line: before.iter().filter(|&&c| c == '\n').count() + 1,
        column: before
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(at + 1, |p| at - p),
    }
}
pub fn detect(text: &str) -> char {
    [',', ';', '\t']
        .into_iter()
        .max_by_key(|&sep| {
            let Ok(table) = parse(text, sep) else {
                return (0, 0, 0);
            };
            let widths: Vec<_> = table.rows.iter().take(50).map(Vec::len).collect();
            let common = widths
                .iter()
                .copied()
                .max_by_key(|w| widths.iter().filter(|x| *x == w).count())
                .unwrap_or(0);
            (
                usize::from(common > 1),
                widths.iter().filter(|&&w| w == common).count(),
                common,
            )
        })
        .filter(|&sep| parse(text, sep).is_ok_and(|t| t.columns > 1))
        .unwrap_or(',')
}
pub fn edit(
    table: &Table,
    row: usize,
    column: usize,
    separator: char,
    value: &str,
) -> Option<ChangeSet> {
    if column >= table.columns {
        return None;
    }
    let cells = table.rows.get(row)?;
    let existing = cells.get(column);
    let quote = existing.is_some_and(|c| c.quoted) || value.contains([separator, '"', '\n', '\r']);
    let encoded = if quote {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    };
    if let Some(cell) = existing {
        Some(ChangeSet::replace(cell.range.0, cell.range.1, encoded))
    } else {
        let at = cells.last()?.range.1;
        let padding: String = std::iter::repeat_n(separator, column + 1 - cells.len()).collect();
        Some(ChangeSet::replace(at, at, format!("{padding}{encoded}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_multiline_and_ragged() {
        let t = parse("name,value,\r\n\"a,b\",\"a\"\"b\nline\"\r\nx\r\n", ',').unwrap();
        assert_eq!(t.columns, 3);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[1][1].value, "a\"b\nline");
        assert_eq!(t.rows[2].len(), 1);
        assert_eq!(t.rows[0][2].value, "");
        assert_eq!(detect("a;b\nx;y\n"), ';');
        assert_eq!(detect("a\tb\nx\ty\n"), '\t');
        assert_eq!(detect("a,b\nx,y\n"), ',');
        assert_eq!(
            parse("a,\"bad", ',').unwrap_err(),
            Error { line: 1, column: 7 }
        );
    }
    #[test]
    fn cell_change_preserves_source() {
        let text = "α,\"old\",\r\nx\r\n";
        let t = parse(text, ',').unwrap();
        let mut rope = crate::Rope::from_str(text);
        let change = edit(&t, 0, 1, ',', "a,\"b").unwrap();
        // Character ranges include the Unicode prefix, never byte offsets.
        assert_eq!(t.rows[0][1].range, (2, 7));
        change.apply(&mut rope);
        assert_eq!(rope.to_string(), "α,\"a,\"\"b\",\r\nx\r\n");
        assert!(edit(&t, 1, 2, ',', "new").is_some());
    }
}
