//! Byte ↔ text conversion (docs/specs/02-document-model.md §Loading): BOM
//! detection, UTF-8 validation, `chardetng` guessing, lossless re-encoding.
//! Pure functions; reading the file is the caller's job.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// How a document's bytes are encoded on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEncoding {
    pub encoding: &'static Encoding,
    pub bom: bool,
}

impl TextEncoding {
    pub const UTF8: TextEncoding = TextEncoding {
        encoding: UTF_8,
        bom: false,
    };

    /// Status-bar label, e.g. `UTF-8`, `UTF-8 BOM`, `UTF-16 LE`, `windows-1252`.
    pub fn label(&self) -> String {
        let name = match self.encoding.name() {
            "UTF-16LE" => "UTF-16 LE",
            "UTF-16BE" => "UTF-16 BE",
            other => other,
        };
        if self.bom && self.encoding == UTF_8 {
            format!("{name} BOM")
        } else {
            name.to_owned()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: TextEncoding,
    /// Replacement characters were introduced: the document must open read-only.
    pub lossy: bool,
}

/// Bytes scanned for NUL / encoding guesses.
const SNIFF_LEN: usize = 8 * 1024;

/// Heuristic binary check: NUL bytes in the first 8 KiB without a UTF-16 BOM.
pub fn looks_binary(bytes: &[u8]) -> bool {
    if bom(bytes).is_some() {
        return false;
    }
    bytes[..bytes.len().min(SNIFF_LEN)].contains(&0)
}

fn bom(bytes: &[u8]) -> Option<(&'static Encoding, usize)> {
    Encoding::for_bom(bytes)
}

/// Decodes `bytes`: BOM → that encoding; valid UTF-8 → UTF-8; else `forced`
/// or a `chardetng` guess.
pub fn decode(bytes: &[u8], forced: Option<&'static Encoding>) -> Decoded {
    if let Some((encoding, bom_len)) = bom(bytes) {
        let (text, lossy) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        return Decoded {
            text: text.into_owned(),
            encoding: TextEncoding {
                encoding,
                bom: true,
            },
            lossy,
        };
    }
    let encoding = match forced {
        Some(e) => e,
        None if std::str::from_utf8(bytes).is_ok() => UTF_8,
        None => {
            let mut detector = chardetng::EncodingDetector::new();
            detector.feed(bytes, true);
            detector.guess(None, true)
        }
    };
    let (text, lossy) = encoding.decode_without_bom_handling(bytes);
    Decoded {
        text: text.into_owned(),
        encoding: TextEncoding {
            encoding,
            bom: false,
        },
        lossy,
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("text contains characters that cannot be represented in {encoding}")]
pub struct Unmappable {
    pub encoding: &'static str,
}

/// Encodes `text` exactly; fails rather than writing replacement characters.
pub fn encode(text: &str, enc: TextEncoding) -> Result<Vec<u8>, Unmappable> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if enc.encoding == UTF_16LE || enc.encoding == UTF_16BE {
        let le = enc.encoding == UTF_16LE;
        if enc.bom {
            out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if le {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        return Ok(out);
    }
    if enc.bom && enc.encoding == UTF_8 {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    let (bytes, _, unmappable) = enc.encoding.encode(text);
    if unmappable {
        return Err(Unmappable {
            encoding: enc.encoding.name(),
        });
    }
    out.extend_from_slice(&bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec 02: "UTF-16 file with BOM round-trips"
    #[test]
    fn utf16le_with_bom_round_trips() {
        let enc = TextEncoding {
            encoding: UTF_16LE,
            bom: true,
        };
        let bytes = encode("héllo", enc).unwrap();
        let decoded = decode(&bytes, None);
        assert_eq!(decoded.text, "héllo");
        assert_eq!(decoded.encoding, enc);
        assert!(!decoded.lossy);
        assert_eq!(
            encode(&format!("{}!", decoded.text), enc).unwrap()[..2],
            [0xFF, 0xFE]
        );
    }

    #[test]
    fn utf8_bom_preserved() {
        let decoded = decode(b"\xEF\xBB\xBFabc", None);
        assert_eq!(decoded.text, "abc");
        assert!(decoded.encoding.bom);
        assert_eq!(decoded.encoding.label(), "UTF-8 BOM");
        assert_eq!(encode("abc", decoded.encoding).unwrap(), b"\xEF\xBB\xBFabc");
    }

    #[test]
    fn latin1_guessed() {
        let decoded = decode(b"caf\xE9 cr\xE8me br\xFBl\xE9e", None);
        assert_eq!(decoded.text, "café crème brûlée");
        assert!(!decoded.lossy);
    }

    #[test]
    fn lossy_decode_is_reported() {
        let decoded = decode(b"ok \xFF\xFE\xFD", Some(UTF_8));
        assert!(decoded.lossy);
    }

    #[test]
    fn unmappable_text_is_an_error() {
        let enc = TextEncoding {
            encoding: encoding_rs::WINDOWS_1252,
            bom: false,
        };
        assert!(encode("日本", enc).is_err());
    }

    #[test]
    fn binary_detection() {
        assert!(looks_binary(b"\x7FELF\x00\x01"));
        assert!(!looks_binary(b"plain text"));
        assert!(!looks_binary(
            &encode(
                "a",
                TextEncoding {
                    encoding: UTF_16LE,
                    bom: true
                }
            )
            .unwrap()
        ));
    }
}
