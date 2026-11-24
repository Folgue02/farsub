//! Text encodings: files open in the encoding they're in and save back in it, so a file
//! that isn't UTF-8 (Latin-1, Windows-1252, Shift_JIS, UTF-16…) is never mangled.
//!
//! Detection: a byte-order mark wins; else valid UTF-8 is UTF-8; else `chardetng` (the
//! detector Firefox uses) guesses. A NUL byte with no UTF-16 BOM means a binary file.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// A file's encoding as FarSub keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEncoding {
    pub encoding: &'static Encoding,
    /// The file starts with a byte-order mark (kept on save).
    pub bom: bool,
}

impl Default for TextEncoding {
    fn default() -> Self {
        TextEncoding {
            encoding: UTF_8,
            bom: false,
        }
    }
}

impl TextEncoding {
    pub fn is_utf8(&self) -> bool {
        self.encoding == UTF_8 && !self.bom
    }

    /// Shown in the status bar ("UTF-8", "windows-1252", "UTF-16LE with BOM").
    pub fn label(&self) -> String {
        let name = self.encoding.name();
        if self.bom && self.encoding == UTF_8 {
            format!("{name} with BOM")
        } else {
            name.to_string()
        }
    }

    /// The file's bytes for `text`. Fails (naming the first characters that don't fit) when
    /// the encoding can't hold some of the text, rather than writing substitutes.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        let utf16 = |be: bool| {
            let mut out = Vec::with_capacity(text.len() * 2 + 2);
            if self.bom {
                out.extend(if be { [0xFE, 0xFF] } else { [0xFF, 0xFE] });
            }
            for u in text.encode_utf16() {
                out.extend(if be { u.to_be_bytes() } else { u.to_le_bytes() });
            }
            out
        };
        if self.encoding == UTF_16LE {
            return Ok(utf16(false));
        }
        if self.encoding == UTF_16BE {
            return Ok(utf16(true));
        }
        if self.encoding == UTF_8 {
            let mut out = Vec::with_capacity(text.len() + 3);
            if self.bom {
                out.extend([0xEF, 0xBB, 0xBF]);
            }
            out.extend(text.as_bytes());
            return Ok(out);
        }
        let (bytes, _, bad) = self.encoding.encode(text);
        if !bad {
            return Ok(bytes.into_owned());
        }
        let missing: Vec<String> = text
            .chars()
            .filter(|c| {
                let mut b = [0; 4];
                self.encoding.encode(c.encode_utf8(&mut b)).2
            })
            .fold(vec![], |mut v: Vec<char>, c| {
                if !v.contains(&c) {
                    v.push(c);
                }
                v
            })
            .into_iter()
            .take(5)
            .map(|c| format!("'{c}'"))
            .collect();
        Err(format!(
            "{} can't hold {} — File: Convert to UTF-8 to save it",
            self.encoding.name(),
            missing.join(", ")
        ))
    }
}

/// Decode a file. `None`: it's binary.
pub fn decode(bytes: &[u8]) -> Option<(String, TextEncoding)> {
    if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        let (text, _) = enc.decode_without_bom_handling(&bytes[bom_len..]);
        return Some((
            text.into_owned(),
            TextEncoding {
                encoding: enc,
                bom: true,
            },
        ));
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some((s.to_string(), TextEncoding::default()));
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(bytes, true);
    let enc = det.guess(None, true);
    let (text, _) = enc.decode_without_bom_handling(bytes);
    Some((
        text.into_owned(),
        TextEncoding {
            encoding: enc,
            bom: false,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_boms_round_trip() {
        let (t, e) = decode("héllo\n".as_bytes()).unwrap();
        assert_eq!((t.as_str(), e.is_utf8()), ("héllo\n", true));
        for raw in [
            b"\xEF\xBB\xBFhi \xC3\xA9".to_vec(),
            b"\xFF\xFEh\x00i\x00 \x00\xE9\x00".to_vec(),
            b"\xFE\xFF\x00h\x00i\x00 \x00\xE9".to_vec(),
        ] {
            let (t, e) = decode(&raw).unwrap();
            assert_eq!(t, "hi é", "{}", e.label());
            assert!(e.bom);
            assert_eq!(e.encode(&t).unwrap(), raw, "{}", e.label());
        }
        assert!(decode(b"ab\x00cd").is_none(), "binary");
    }

    #[test]
    fn legacy_encodings_are_detected_and_kept() {
        // "Café crème à côté" in Windows-1252 / Latin-1: not valid UTF-8.
        let raw = b"Caf\xE9 cr\xE8me \xE0 c\xF4t\xE9, d\xE9j\xE0 vu\n".to_vec();
        let (t, e) = decode(&raw).unwrap();
        assert_eq!(t, "Café crème à côté, déjà vu\n");
        assert!(!e.is_utf8());
        assert_eq!(e.encode(&t).unwrap(), raw, "saved byte for byte");
        // Text the encoding can't hold is refused, not written as '?' or "&#...;".
        let err = e.encode("Café 日本").unwrap_err();
        assert!(
            err.contains("'日'") && err.contains("Convert to UTF-8"),
            "{err}"
        );
    }
}
