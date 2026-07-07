use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::str::Utf8Error;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// An error while decoding Python source bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceDecodeError {
    /// The source begins with more than one UTF-8 byte-order mark.
    MultipleUtf8Boms,
    /// The source declares an encoding that Serc does not support.
    UnsupportedEncoding(String),
    /// A UTF-8 byte-order mark conflicts with the declared encoding.
    Utf8BomConflict(String),
    /// The source is not valid UTF-8.
    InvalidUtf8(Utf8Error),
    /// The source contains a null byte.
    NullByte,
}

impl fmt::Display for SourceDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultipleUtf8Boms => formatter.write_str("source contains multiple UTF-8 BOMs"),
            Self::UnsupportedEncoding(encoding) => {
                write!(formatter, "unsupported source encoding `{encoding}`")
            }
            Self::Utf8BomConflict(encoding) => {
                write!(
                    formatter,
                    "encoding problem: UTF-8 BOM conflicts with `{encoding}`"
                )
            }
            Self::InvalidUtf8(error) => write!(formatter, "source is not valid UTF-8: {error}"),
            Self::NullByte => formatter.write_str("source code string cannot contain null bytes"),
        }
    }
}

impl Error for SourceDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Utf8Encoding {
    BomCompatible,
    Alias,
}

/// Decode Python source bytes and normalize them for parsing.
///
/// Serc accepts UTF-8 source, including a single UTF-8 BOM and CPython-compatible UTF-8
/// encoding cookies on the first or second physical line. Newlines are normalized to `\n`. The
/// returned value borrows `bytes` when no newline normalization is required.
pub fn decode_source(bytes: &[u8]) -> Result<Cow<'_, str>, SourceDecodeError> {
    let (bytes, has_bom) = bytes
        .strip_prefix(UTF8_BOM)
        .map_or((bytes, false), |bytes| (bytes, true));
    if has_bom && bytes.starts_with(UTF8_BOM) {
        return Err(SourceDecodeError::MultipleUtf8Boms);
    }
    if let Some(cookie) = declared_encoding(bytes) {
        let Some(encoding) = classify_utf8_encoding(cookie) else {
            return Err(SourceDecodeError::UnsupportedEncoding(cookie.to_string()));
        };
        if has_bom && encoding == Utf8Encoding::Alias {
            return Err(SourceDecodeError::Utf8BomConflict(cookie.to_string()));
        }
    }

    let source = std::str::from_utf8(bytes).map_err(SourceDecodeError::InvalidUtf8)?;
    if source.contains('\0') {
        return Err(SourceDecodeError::NullByte);
    }
    Ok(normalize_newlines(source))
}

pub(crate) fn normalize_source(source: &str) -> Cow<'_, str> {
    normalize_newlines(source.strip_prefix('\u{feff}').unwrap_or(source))
}

fn normalize_newlines(source: &str) -> Cow<'_, str> {
    if !source.as_bytes().contains(&b'\r') {
        return Cow::Borrowed(source);
    }

    let mut normalized = String::with_capacity(source.len());
    let mut copied = 0;
    let mut index = 0;
    let bytes = source.as_bytes();
    while index < bytes.len() {
        if bytes[index] != b'\r' {
            index += 1;
            continue;
        }
        normalized.push_str(&source[copied..index]);
        normalized.push('\n');
        index += 1;
        if bytes.get(index) == Some(&b'\n') {
            index += 1;
        }
        copied = index;
    }
    normalized.push_str(&source[copied..]);
    Cow::Owned(normalized)
}

fn declared_encoding(bytes: &[u8]) -> Option<&str> {
    let first_end = physical_line_end(bytes);
    let first = &bytes[..first_end];
    if let Some(cookie) = encoding_cookie(first) {
        return Some(cookie);
    }
    if !is_blank_or_comment(first) || first_end == bytes.len() {
        return None;
    }

    let remaining = &bytes[first_end..];
    let second_end = physical_line_end(remaining);
    encoding_cookie(&remaining[..second_end])
}

fn physical_line_end(bytes: &[u8]) -> usize {
    for (index, byte) in bytes.iter().copied().enumerate() {
        match byte {
            b'\n' => return index + 1,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => return index + 2,
            b'\r' => return index + 1,
            _ => {}
        }
    }
    bytes.len()
}

fn encoding_cookie(line: &[u8]) -> Option<&str> {
    let comment_start = line
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | 0x0c))?;
    if line[comment_start] != b'#' {
        return None;
    }
    let comment = &line[comment_start + 1..];

    for index in 0..comment.len().saturating_sub(6) {
        if &comment[index..index + 6] != b"coding" || !matches!(comment[index + 6], b':' | b'=') {
            continue;
        }
        let mut start = index + 7;
        while matches!(comment.get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        let mut end = start;
        while comment
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            end += 1;
        }
        if start < end {
            return std::str::from_utf8(&comment[start..end]).ok();
        }
    }
    None
}

fn is_blank_or_comment(line: &[u8]) -> bool {
    line.iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | 0x0c))
        .is_none_or(|byte| matches!(byte, b'#' | b'\r' | b'\n'))
}

fn classify_utf8_encoding(name: &str) -> Option<Utf8Encoding> {
    let normalized_prefix: String = name
        .bytes()
        .take(12)
        .map(|byte| match byte {
            b'A'..=b'Z' => char::from(byte.to_ascii_lowercase()),
            b'_' => '-',
            _ => char::from(byte),
        })
        .collect();
    if normalized_prefix == "utf-8" || normalized_prefix.starts_with("utf-8-") {
        return Some(Utf8Encoding::BomCompatible);
    }

    matches!(
        normalize_codec_name(name).as_str(),
        "utf" | "u8" | "utf8" | "utf8_ucs2" | "utf8_ucs4" | "cp65001"
    )
    .then_some(Utf8Encoding::Alias)
}

fn normalize_codec_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut separator = false;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'.' {
            if separator && !normalized.is_empty() {
                normalized.push('_');
            }
            normalized.push(char::from(byte.to_ascii_lowercase()));
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{SourceDecodeError, declared_encoding, decode_source, normalize_source};

    #[test]
    fn borrows_source_without_normalization() {
        assert!(matches!(
            decode_source(b"pass\n"),
            Ok(Cow::Borrowed("pass\n"))
        ));
        assert!(matches!(
            normalize_source("pass\n"),
            Cow::Borrowed("pass\n")
        ));
    }

    #[test]
    fn normalizes_newlines_in_one_pass() {
        assert_eq!(
            decode_source(b"first\r\nsecond\rthird\n").unwrap(),
            "first\nsecond\nthird\n"
        );
    }

    #[test]
    fn limits_encoding_cookies_to_two_physical_lines() {
        assert_eq!(
            declared_encoding(b"# first\r# coding: latin-1\rvalue = '\xe9'\r"),
            Some("latin-1")
        );
        assert_eq!(
            declared_encoding(b"# first\r# second\r# coding: latin-1\r"),
            None
        );
    }

    #[test]
    fn rejects_invalid_source_bytes() {
        assert_eq!(
            decode_source(b"\xef\xbb\xbf\xef\xbb\xbfpass\n"),
            Err(SourceDecodeError::MultipleUtf8Boms)
        );
        assert!(matches!(
            decode_source(b"value = '\xff'\n"),
            Err(SourceDecodeError::InvalidUtf8(_))
        ));
        assert_eq!(
            decode_source(b"value = 'before\0after'\n"),
            Err(SourceDecodeError::NullByte)
        );
        assert_eq!(
            decode_source(b"\xef\xbb\xbf# coding: utf8\npass\n"),
            Err(SourceDecodeError::Utf8BomConflict("utf8".to_string()))
        );
    }

    #[test]
    fn accepts_supported_utf8_aliases() {
        for encoding in [
            "utf-8",
            "UTF_8",
            "utf-8-sig",
            "utf",
            "u8",
            "utf8",
            "utf8_ucs2",
            "utf8_ucs4",
            "cp65001",
        ] {
            let source = format!("# coding: {encoding}\npass\n");
            assert!(decode_source(source.as_bytes()).is_ok(), "{encoding}");
        }
    }

    #[test]
    fn recognizes_second_line_cookies_with_every_newline_form() {
        for newline in ["\n", "\r\n", "\r"] {
            let supported = format!("# first{newline}# coding: utf8{newline}pass{newline}");
            assert!(decode_source(supported.as_bytes()).is_ok(), "{newline:?}");

            let unsupported = format!("# first{newline}# coding: latin-1{newline}pass{newline}");
            assert_eq!(
                decode_source(unsupported.as_bytes()),
                Err(SourceDecodeError::UnsupportedEncoding(
                    "latin-1".to_string()
                )),
                "{newline:?}",
            );
        }
    }
}
