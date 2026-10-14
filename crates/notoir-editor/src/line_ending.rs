//! Line endings (AD-13): buffers hold `\n` only; the original style is a
//! document property restored on save.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }

    /// Default for new files on this platform.
    pub fn platform_default() -> Self {
        if cfg!(windows) {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EolInfo {
    pub ending: LineEnding,
    pub mixed: bool,
}

/// Majority style wins; ties and files without line breaks use `default`.
pub fn detect(text: &str, default: LineEnding) -> EolInfo {
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [
        (LineEnding::Lf, lf),
        (LineEnding::CrLf, crlf),
        (LineEnding::Cr, cr),
    ];
    let present = kinds.iter().filter(|(_, n)| *n > 0).count();
    let max = kinds.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let winners: Vec<LineEnding> = kinds
        .iter()
        .filter(|(_, n)| *n == max && max > 0)
        .map(|(k, _)| *k)
        .collect();
    let ending = match winners.as_slice() {
        [single] => *single,
        _ => default,
    };
    EolInfo {
        ending,
        mixed: present > 1,
    }
}

/// Converts every `\r\n` and lone `\r` to `\n`.
pub fn normalize(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_owned();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Converts `\n` to `ending` (input must be normalized).
pub fn denormalize(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Lf => text.to_owned(),
        other => text.replace('\n', other.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec 02: "CRLF preserved"
    #[test]
    fn crlf_detected_and_round_trips() {
        let text = "a\r\nb\r\nc";
        let info = detect(text, LineEnding::Lf);
        assert_eq!(
            info,
            EolInfo {
                ending: LineEnding::CrLf,
                mixed: false
            }
        );
        let norm = normalize(text);
        assert_eq!(norm, "a\nb\nc");
        assert_eq!(denormalize(&norm, info.ending), text);
    }

    // spec 02: "Mixed line endings are reported, not silently changed"
    #[test]
    fn mixed_line_endings_report_majority() {
        let text = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\r\n12\r\n";
        let info = detect(text, LineEnding::CrLf);
        assert_eq!(
            info,
            EolInfo {
                ending: LineEnding::Lf,
                mixed: true
            }
        );
    }

    #[test]
    fn no_line_breaks_use_default() {
        assert_eq!(detect("abc", LineEnding::CrLf).ending, LineEnding::CrLf);
    }

    #[test]
    fn lone_cr_normalized() {
        assert_eq!(normalize("a\rb"), "a\nb");
    }
}
