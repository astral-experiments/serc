use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::str::Utf8Error;

use encoding_rs::{BIG5, EUC_JP, Encoding, ISO_8859_15, KOI8_R, WINDOWS_1251, WINDOWS_1252};

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
    /// The source is not valid in its declared encoding.
    InvalidEncoding(String),
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
            Self::InvalidEncoding(encoding) => {
                write!(formatter, "source is not valid `{encoding}`")
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
enum SourceEncoding {
    Utf8(Utf8Encoding),
    Ascii,
    Latin1,
    Iso885915,
    Big5,
    EucJp,
    Koi8R,
    Windows1251,
    Windows1252,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Utf8Encoding {
    BomCompatible,
    Alias,
}

/// Standard-library modules that CPython imports while decoding byte source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceCodec {
    Ascii,
    Latin1,
    Iso885915,
    Big5,
    EucJp,
    Koi8R,
    Windows1251,
    Windows1252,
}

pub(crate) struct DecodedSource<'a> {
    pub(crate) source: Cow<'a, str>,
    pub(crate) codec: Option<SourceCodec>,
    pub(crate) registry_name: Option<String>,
}

/// Decode Python source bytes and normalize them for parsing.
///
/// Serc accepts UTF-8 source, including a single UTF-8 BOM and CPython-compatible UTF-8
/// encoding cookies on the first or second physical line. ASCII, Latin-1, Latin-9, Big5, EUC-JP, KOI8-R, and
/// Windows 1251 and 1252 cookies are also decoded with CPython-compatible aliases and strict mappings.
/// Newlines are normalized to `\n`. The returned value borrows `bytes` for unchanged UTF-8 source.
pub fn decode_source(bytes: &[u8]) -> Result<Cow<'_, str>, SourceDecodeError> {
    Ok(decode_source_with_codec(bytes)?.source)
}

pub(crate) fn decode_source_with_codec(
    bytes: &[u8],
) -> Result<DecodedSource<'_>, SourceDecodeError> {
    if bytes.contains(&0) {
        return Err(SourceDecodeError::NullByte);
    }
    let (bytes, has_bom) = bytes
        .strip_prefix(UTF8_BOM)
        .map_or((bytes, false), |bytes| (bytes, true));
    if has_bom && bytes.starts_with(UTF8_BOM) {
        return Err(SourceDecodeError::MultipleUtf8Boms);
    }
    let declared = declared_encoding(bytes);
    let encoding = match declared {
        Some(cookie) => {
            let Some(encoding) = classify_source_encoding(cookie) else {
                return Err(SourceDecodeError::UnsupportedEncoding(cookie.to_string()));
            };
            if has_bom && encoding != SourceEncoding::Utf8(Utf8Encoding::BomCompatible) {
                return Err(SourceDecodeError::Utf8BomConflict(cookie.to_string()));
            }
            encoding
        }
        None => SourceEncoding::Utf8(Utf8Encoding::BomCompatible),
    };

    let source = match encoding {
        SourceEncoding::Ascii => {
            if !bytes.is_ascii() {
                return Err(SourceDecodeError::InvalidEncoding(
                    declared
                        .expect("ASCII source encoding has a cookie")
                        .to_string(),
                ));
            }
            Cow::Borrowed(std::str::from_utf8(bytes).expect("ASCII is valid UTF-8"))
        }
        SourceEncoding::Utf8(_) => {
            Cow::Borrowed(std::str::from_utf8(bytes).map_err(SourceDecodeError::InvalidUtf8)?)
        }
        SourceEncoding::Latin1 => Cow::Owned(bytes.iter().copied().map(char::from).collect()),
        SourceEncoding::Iso885915
        | SourceEncoding::Big5
        | SourceEncoding::EucJp
        | SourceEncoding::Koi8R
        | SourceEncoding::Windows1251
        | SourceEncoding::Windows1252 => {
            let encoding_name = declared.expect("legacy source encoding has a cookie");
            decode_legacy(encoding, bytes)
                .ok_or_else(|| SourceDecodeError::InvalidEncoding(encoding_name.to_string()))?
        }
    };
    let (codec, registry_name) =
        declared.map_or((None, None), |name| codec_registry_effects(name, encoding));
    Ok(DecodedSource {
        source: normalize_decoded_source(source),
        codec,
        registry_name,
    })
}

/// Mirror the tokenizer's UTF-8/Latin-1 normalization and Unicode decoder shortcuts.
/// Successful codec-registry lookups retain the normalized requested name as well
/// as the standard codec module's code and dictionary keys.
fn codec_registry_effects(
    name: &str,
    encoding: SourceEncoding,
) -> (Option<SourceCodec>, Option<String>) {
    let normalized = normalize_codec_name(name);
    let fast_path = match encoding {
        SourceEncoding::Utf8(Utf8Encoding::BomCompatible) => true,
        SourceEncoding::Utf8(Utf8Encoding::Alias) => normalized == "utf8" || normalized == "utf_8",
        SourceEncoding::Ascii => matches!(normalized.as_str(), "ascii" | "us_ascii"),
        SourceEncoding::Latin1 => {
            classify_latin1_encoding(name)
                || matches!(
                    normalized.as_str(),
                    "latin1" | "latin_1" | "iso_8859_1" | "iso8859_1"
                )
        }
        SourceEncoding::Iso885915
        | SourceEncoding::Big5
        | SourceEncoding::EucJp
        | SourceEncoding::Koi8R
        | SourceEncoding::Windows1251
        | SourceEncoding::Windows1252 => false,
    };
    if fast_path {
        return (None, None);
    }
    let codec = match encoding {
        // encodings.utf_8 is loaded during normal CPython startup; the caller's
        // pre-decoding state already describes the module's retained strings.
        SourceEncoding::Utf8(_) => None,
        SourceEncoding::Ascii => Some(SourceCodec::Ascii),
        SourceEncoding::Latin1 => Some(SourceCodec::Latin1),
        SourceEncoding::Iso885915 => Some(SourceCodec::Iso885915),
        SourceEncoding::Big5 => Some(SourceCodec::Big5),
        SourceEncoding::EucJp => Some(SourceCodec::EucJp),
        SourceEncoding::Koi8R => Some(SourceCodec::Koi8R),
        SourceEncoding::Windows1251 => Some(SourceCodec::Windows1251),
        SourceEncoding::Windows1252 => Some(SourceCodec::Windows1252),
    };
    (codec, Some(normalized))
}

fn decode_legacy(encoding: SourceEncoding, bytes: &[u8]) -> Option<Cow<'_, str>> {
    match encoding {
        SourceEncoding::Iso885915 => {
            ISO_8859_15.decode_without_bom_handling_and_without_replacement(bytes)
        }
        SourceEncoding::Big5 => decode_multibyte(bytes, BIG5, big5_character),
        SourceEncoding::EucJp => decode_multibyte(bytes, EUC_JP, euc_jp_character),
        SourceEncoding::Koi8R => KOI8_R.decode_without_bom_handling_and_without_replacement(bytes),
        SourceEncoding::Windows1251 if !bytes.contains(&0x98) => {
            WINDOWS_1251.decode_without_bom_handling_and_without_replacement(bytes)
        }
        // The web decoder maps these undefined bytes to C1 controls, while
        // CPython's strict Windows-1252 charmap rejects them.
        SourceEncoding::Windows1252
            if !bytes
                .iter()
                .any(|byte| matches!(byte, 0x81 | 0x8d | 0x8f | 0x90 | 0x9d)) =>
        {
            WINDOWS_1252.decode_without_bom_handling_and_without_replacement(bytes)
        }
        SourceEncoding::Windows1251 | SourceEncoding::Windows1252 => None,
        SourceEncoding::Utf8(_) | SourceEncoding::Ascii | SourceEncoding::Latin1 => unreachable!(),
    }
}

/// Decode the common mappings in runs. Each character is first checked against
/// CPython's repertoire, which omits web extensions and maps some bytes differently.
fn decode_multibyte<'a>(
    bytes: &'a [u8],
    encoding: &'static Encoding,
    character: fn(&[u8], usize) -> Option<(usize, Option<char>)>,
) -> Option<Cow<'a, str>> {
    let mut output = None;
    let mut segment_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let (length, replacement) = character(bytes, index)?;
        if let Some(replacement) = replacement {
            let text = output.get_or_insert_with(|| String::with_capacity(bytes.len()));
            let segment = encoding.decode_without_bom_handling_and_without_replacement(
                &bytes[segment_start..index],
            )?;
            text.push_str(&segment);
            text.push(replacement);
            segment_start = index + length;
        }
        index += length;
    }
    let tail =
        encoding.decode_without_bom_handling_and_without_replacement(&bytes[segment_start..])?;
    if let Some(mut output) = output {
        output.push_str(&tail);
        Some(Cow::Owned(output))
    } else {
        Some(tail)
    }
}

fn big5_character(bytes: &[u8], index: usize) -> Option<(usize, Option<char>)> {
    let lead = bytes[index];
    if lead.is_ascii() {
        return Some((1, None));
    }
    let trail = *bytes.get(index + 1)?;
    let last_trail = match lead {
        0xa1..=0xa2 | 0xa4..=0xc6 | 0xc9..=0xf8 => 0xfe,
        0xa3 => 0xbf,
        0xc7 => 0xfc,
        0xf9 => 0xd5,
        _ => return None,
    };
    if !(0x40..=0x7e).contains(&trail) && !(0xa1..=last_trail).contains(&trail) {
        return None;
    }
    Some((2, big5_override(u16::from_be_bytes([lead, trail]))))
}

/// CPython uses the Big5 punctuation, kana, Cyrillic and enclosed-number rows
/// where the WHATWG index uses different characters.
fn big5_override(pair: u16) -> Option<char> {
    let (first, codepoint) = match pair {
        0xa145 => return Some('\u{2022}'),
        0xa14e => return Some('\u{ff64}'),
        0xa1c2 => return Some('\u{203e}'),
        0xa1e3 => return Some('\u{223c}'),
        0xa1f2 => return Some('\u{2641}'),
        0xa1f3 => return Some('\u{2609}'),
        0xa241 => return Some('\u{ff0f}'),
        0xa242 => return Some('\u{ff3c}'),
        0xa244 => return Some('\u{00a5}'),
        0xa246 => return Some('\u{00a2}'),
        0xa247 => return Some('\u{00a3}'),
        0xc6a1 => return Some('\u{30fe}'),
        0xc6a2..=0xc6a3 => (0xc6a2, 0x309d),
        0xc6a4 => return Some('\u{3005}'),
        0xc6a5..=0xc6f7 => (0xc6a5, 0x3041),
        0xc6f8..=0xc6fe => (0xc6f8, 0x30a1),
        0xc740..=0xc77e => (0xc740, 0x30a8),
        0xc7a1..=0xc7b0 => (0xc7a1, 0x30e7),
        0xc7b1..=0xc7b2 => (0xc7b1, 0x0414),
        0xc7b3 => return Some('\u{0401}'),
        0xc7b4..=0xc7ba => (0xc7b4, 0x0416),
        0xc7bb..=0xc7cd => (0xc7bb, 0x0423),
        0xc7ce => return Some('\u{0451}'),
        0xc7cf..=0xc7e8 => (0xc7cf, 0x0436),
        0xc7e9..=0xc7f2 => (0xc7e9, 0x2460),
        0xc7f3..=0xc7fc => (0xc7f3, 0x2474),
        _ => return None,
    };
    char::from_u32(codepoint + u32::from(pair - first))
}

fn euc_jp_character(bytes: &[u8], index: usize) -> Option<(usize, Option<char>)> {
    let length = euc_jp_character_len(bytes, index)?;
    let sequence = &bytes[index..index + length];
    if length == 2 && euc_jp_web_extension(sequence[0], sequence[1]) {
        return None;
    }
    Some((length, euc_jp_override(sequence)))
}

fn euc_jp_character_len(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes[index] {
        0x00..=0x7f => Some(1),
        0x8e if matches!(bytes.get(index + 1), Some(0xa1..=0xdf)) => Some(2),
        0x8f if matches!(bytes.get(index + 1), Some(0xa1..=0xfe))
            && matches!(bytes.get(index + 2), Some(0xa1..=0xfe)) =>
        {
            Some(3)
        }
        0xa1..=0xfe if matches!(bytes.get(index + 1), Some(0xa1..=0xfe)) => Some(2),
        _ => None,
    }
}

fn euc_jp_web_extension(lead: u8, trail: u8) -> bool {
    match lead {
        0xad => matches!(trail, 0xa1..=0xbe | 0xc0..=0xd6 | 0xdf..=0xfc),
        0xf9..=0xfb => matches!(trail, 0xa1..=0xfe),
        0xfc => matches!(trail, 0xa1..=0xee | 0xf1..=0xfe),
        _ => false,
    }
}

fn euc_jp_override(sequence: &[u8]) -> Option<char> {
    match sequence {
        [0xa1, 0xc1] => Some('\u{301c}'),
        [0xa1, 0xc2] => Some('\u{2016}'),
        [0xa1, 0xdd] => Some('\u{2212}'),
        [0xa1, 0xf1] => Some('\u{00a2}'),
        [0xa1, 0xf2] => Some('\u{00a3}'),
        [0xa2, 0xcc] => Some('\u{00ac}'),
        [0x8f, 0xa2, 0xb7] => Some('~'),
        _ => None,
    }
}

fn normalize_decoded_source(source: Cow<'_, str>) -> Cow<'_, str> {
    match source {
        Cow::Borrowed(source) => normalize_newlines(source),
        Cow::Owned(source) => {
            if source.as_bytes().contains(&b'\r') {
                Cow::Owned(normalize_newlines(&source).into_owned())
            } else {
                Cow::Owned(source)
            }
        }
    }
}

fn classify_source_encoding(name: &str) -> Option<SourceEncoding> {
    if let Some(encoding) = classify_utf8_encoding(name) {
        return Some(SourceEncoding::Utf8(encoding));
    }
    if classify_latin1_encoding(name) {
        return Some(SourceEncoding::Latin1);
    }
    let normalized = normalize_codec_name(name);
    let encoding = match normalized.as_str() {
        "ascii" | "646" | "ansi_x3.4_1968" | "ansi_x3_4_1968" | "ansi_x3.4_1986" | "cp367"
        | "csascii" | "ibm367" | "iso646_us" | "iso_646.irv_1991" | "iso_ir_6" | "us"
        | "us_ascii" => Some(SourceEncoding::Ascii),
        "8859" | "cp819" | "csisolatin1" | "ibm819" | "iso8859" | "iso8859_1" | "iso_8859_1"
        | "iso_8859_1_1987" | "iso_ir_100" | "l1" | "latin" | "latin1" | "latin_1" => {
            Some(SourceEncoding::Latin1)
        }
        "iso8859_15" | "iso_8859_15" | "l9" | "latin9" => Some(SourceEncoding::Iso885915),
        "big5" | "big5_tw" | "csbig5" | "x_mac_trad_chinese" => Some(SourceEncoding::Big5),
        "euc_jp" | "eucjp" | "ujis" | "u_jis" => Some(SourceEncoding::EucJp),
        "koi8_r" | "cskoi8r" => Some(SourceEncoding::Koi8R),
        "windows_1251" | "cp1251" | "1251" => Some(SourceEncoding::Windows1251),
        "windows_1252" | "cp1252" | "1252" => Some(SourceEncoding::Windows1252),
        _ => None,
    };
    encoding.or_else(|| classify_dotted_codec_alias(&normalized))
}

fn classify_dotted_codec_alias(name: &str) -> Option<SourceEncoding> {
    let alias = name.contains('.').then(|| name.replace('.', "_"))?;
    match alias.as_str() {
        "ansi_x3_4_1968" | "iso_ir_6" | "us_ascii" | "iso646_us" => Some(SourceEncoding::Ascii),
        "iso8859_1" | "iso_8859_1" | "iso_8859_1_1987" | "iso_ir_100" => {
            Some(SourceEncoding::Latin1)
        }
        "iso8859_15" | "iso_8859_15" => Some(SourceEncoding::Iso885915),
        "big5_tw" | "x_mac_trad_chinese" => Some(SourceEncoding::Big5),
        "u_jis" => Some(SourceEncoding::EucJp),
        "windows_1251" => Some(SourceEncoding::Windows1251),
        "windows_1252" => Some(SourceEncoding::Windows1252),
        _ => None,
    }
}

fn classify_latin1_encoding(name: &str) -> bool {
    let normalized_prefix = normalize_encoding_prefix(name);
    ["latin-1", "iso-8859-1", "iso-latin-1"]
        .iter()
        .any(|prefix| {
            normalized_prefix == *prefix
                || normalized_prefix
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('-'))
        })
}

pub(crate) fn normalize_source(source: &str) -> Cow<'_, str> {
    normalize_newlines(source)
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
    let normalized_prefix = normalize_encoding_prefix(name);
    if normalized_prefix == "utf-8" || normalized_prefix.starts_with("utf-8-") {
        return Some(Utf8Encoding::BomCompatible);
    }

    let normalized = normalize_codec_name(name);
    if matches!(
        normalized.as_str(),
        "utf" | "u8" | "utf8" | "utf_8" | "utf_8_sig" | "utf8_ucs2" | "utf8_ucs4" | "cp65001"
    ) {
        return Some(Utf8Encoding::Alias);
    }
    normalized
        .contains('.')
        .then(|| normalized.replace('.', "_"))
        .is_some_and(|alias| matches!(alias.as_str(), "utf8_ucs2" | "utf8_ucs4"))
        .then_some(Utf8Encoding::Alias)
}

/// Normalize the first 12 bytes for CPython's tokenizer encoding-prefix checks.
///
/// These checks map underscores to hyphens and preserve other punctuation, unlike
/// the full codec-name normalization used when looking up aliases.
fn normalize_encoding_prefix(name: &str) -> String {
    name.bytes()
        .take(12)
        .map(|byte| match byte {
            b'A'..=b'Z' => char::from(byte.to_ascii_lowercase()),
            b'_' => '-',
            _ => char::from(byte),
        })
        .collect()
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

    use super::{
        SourceCodec, SourceDecodeError, SourceEncoding, classify_source_encoding,
        declared_encoding, decode_legacy, decode_source, decode_source_with_codec,
        normalize_source,
    };

    #[test]
    fn codec_effects_distinguish_decoder_shortcuts_and_registry_aliases() {
        for name in [
            "utf-8",
            "utf_8_sig",
            "utf8",
            "ascii",
            "us_ascii",
            "latin1",
            "latin_1",
            "iso_8859_1_1987",
        ] {
            let source = format!("# coding: {name}\npass\n");
            let decoded = decode_source_with_codec(source.as_bytes()).unwrap();
            assert_eq!(
                (decoded.codec, decoded.registry_name),
                (None, None),
                "{name}"
            );
        }
        for (name, codec, registry_name) in [
            ("utf", None, "utf"),
            ("cp65001", None, "cp65001"),
            ("cp367", Some(SourceCodec::Ascii), "cp367"),
            ("cp819", Some(SourceCodec::Latin1), "cp819"),
            ("u.jis", Some(SourceCodec::EucJp), "u.jis"),
            (
                "windows.1251",
                Some(SourceCodec::Windows1251),
                "windows.1251",
            ),
            ("koi8-r", Some(SourceCodec::Koi8R), "koi8_r"),
            ("latin9", Some(SourceCodec::Iso885915), "latin9"),
        ] {
            let source = format!("# coding: {name}\npass\n");
            let decoded = decode_source_with_codec(source.as_bytes()).unwrap();
            assert_eq!(
                (decoded.codec, decoded.registry_name.as_deref()),
                (codec, Some(registry_name)),
                "{name}"
            );
        }
        let decoded = decode_source_with_codec(b"pass\n").unwrap();
        assert_eq!((decoded.codec, decoded.registry_name), (None, None));
    }

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
    fn ascii_source_cookies_are_strict_and_preserve_borrowing() {
        for alias in [
            "ascii",
            "US-ASCII",
            "646",
            "ANSI_X3.4-1968",
            "iso_646.irv_1991",
        ] {
            let source = format!("# coding: {alias}\nvalue = 'text'\n");
            assert!(matches!(
                decode_source(source.as_bytes()),
                Ok(Cow::Borrowed(_))
            ));
            assert_eq!(decode_source(source.as_bytes()).unwrap(), source);
        }
        assert_eq!(
            decode_source(b"# coding: ascii\r\nvalue = 1\r").unwrap(),
            "# coding: ascii\nvalue = 1\n"
        );
        for bytes in [
            b"# coding: ascii\nvalue = '\xff'\n".as_slice(),
            b"# coding: ascii\nvalue = '\xc3\xa9'\n".as_slice(),
        ] {
            assert_eq!(
                decode_source(bytes),
                Err(SourceDecodeError::InvalidEncoding("ascii".to_string()))
            );
        }
        assert_eq!(
            decode_source(b"\xef\xbb\xbf# coding: ascii\npass\n"),
            Err(SourceDecodeError::Utf8BomConflict("ascii".to_string()))
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
    fn decodes_supported_legacy_encodings_and_normalizes_newlines() {
        assert_eq!(
            decode_source(b"# coding: latin-1\r\nvalue = '\xe9'\r").unwrap(),
            "# coding: latin-1\nvalue = '\u{e9}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: iso-8859-15\nvalue = '\xa4\xa6\xa8\xb4\xb8\xbc\xbd\xbe'\n")
                .unwrap(),
            "# coding: iso-8859-15\nvalue = '\u{20ac}\u{0160}\u{0161}\u{017d}\u{017e}\u{0152}\u{0153}\u{0178}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: euc-jp\nvalue = '\xa4\xa2'\n").unwrap(),
            "# coding: euc-jp\nvalue = '\u{3042}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: koi8-r\nvalue = '\xe1'\n").unwrap(),
            "# coding: koi8-r\nvalue = '\u{410}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: cp1251\nvalue = '\xc0'\n").unwrap(),
            "# coding: cp1251\nvalue = '\u{410}'\n"
        );
        assert_eq!(
            decode_source(b"# coding: latin-1\nvalue = '\0'\n"),
            Err(SourceDecodeError::NullByte)
        );
    }

    #[test]
    fn rejects_invalid_source_bytes() {
        for undefined in [0x81, 0x8d, 0x8f, 0x90, 0x9d] {
            let mut source = b"# coding: cp1252\n# ".to_vec();
            source.push(undefined);
            assert_eq!(
                decode_source(&source),
                Err(SourceDecodeError::InvalidEncoding("cp1252".to_string()))
            );
        }
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
        assert_eq!(
            decode_source(b"\xef\xbb\xbf# coding: latin-1\npass\n"),
            Err(SourceDecodeError::Utf8BomConflict("latin-1".to_string()))
        );
        assert_eq!(
            decode_source(b"# coding: cp1251\nvalue = '\x98'\n"),
            Err(SourceDecodeError::InvalidEncoding("cp1251".to_string()))
        );
        assert_eq!(
            decode_source(b"# coding: euc-jp\nvalue = '\xa4'\n"),
            Err(SourceDecodeError::InvalidEncoding("euc-jp".to_string()))
        );
        assert_eq!(
            decode_source(b"# coding: euc-jp\nvalue = '\xad\xa1'\n"),
            Err(SourceDecodeError::InvalidEncoding("euc-jp".to_string()))
        );
        assert_eq!(
            decode_source(b"# coding: koi8_u\npass\n"),
            Err(SourceDecodeError::UnsupportedEncoding("koi8_u".to_string()))
        );
        for source in [
            b"\xff\0".as_slice(),
            b"# coding: cp1251\n\x98\0",
            b"# coding: big5\n\0",
            b"\xef\xbb\xbf\xef\xbb\xbf\0",
        ] {
            assert_eq!(decode_source(source), Err(SourceDecodeError::NullByte));
        }
    }

    #[test]
    fn decodes_cpython_big5_rows_and_rejects_extensions() {
        for encoding in [
            "big5",
            "big5-tw",
            "csbig5",
            "x-mac-trad-chinese",
            "big5.tw",
            "x.mac.trad.chinese",
        ] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::Big5)
            );
        }
        assert_eq!(
            decode_legacy(
                SourceEncoding::Big5,
                b"a\xa1\x45\xa4\x40\xc6\xa5\xc7\xb3\xc7\xe9z"
            )
            .unwrap(),
            "a•一ぁЁ①z"
        );
        for invalid in [
            &[0xa4][..],
            &[0x81, 0x40],
            &[0xa3, 0xc0],
            &[0xc7, 0xfd],
            &[0xc8, 0x40],
            &[0xf9, 0xd6],
            &[0xa4, 0x7f],
        ] {
            assert!(decode_legacy(SourceEncoding::Big5, invalid).is_none());
        }
    }

    #[test]
    fn applies_cpython_euc_jp_mapping_overrides() {
        for (encoded, expected) in [
            (b"\xa1\xc1".as_slice(), '\u{301c}'),
            (b"\xa1\xc2".as_slice(), '\u{2016}'),
            (b"\xa1\xdd".as_slice(), '\u{2212}'),
            (b"\xa1\xf1".as_slice(), '\u{00a2}'),
            (b"\xa1\xf2".as_slice(), '\u{00a3}'),
            (b"\xa2\xcc".as_slice(), '\u{00ac}'),
            (b"\x8f\xa2\xb7".as_slice(), '~'),
        ] {
            assert_eq!(
                decode_legacy(SourceEncoding::EucJp, encoded).unwrap(),
                expected.to_string()
            );
        }
    }

    #[test]
    fn rejects_euc_jp_web_extension_ranges() {
        for (lead, ranges) in [
            (0xad, &[(0xa1, 0xbe), (0xc0, 0xd6), (0xdf, 0xfc)][..]),
            (0xf9, &[(0xa1, 0xfe)][..]),
            (0xfa, &[(0xa1, 0xfe)][..]),
            (0xfb, &[(0xa1, 0xfe)][..]),
            (0xfc, &[(0xa1, 0xee), (0xf1, 0xfe)][..]),
        ] {
            for &(start, end) in ranges {
                for trail in start..=end {
                    assert!(decode_legacy(SourceEncoding::EucJp, &[lead, trail]).is_none());
                }
            }
        }
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
            "utf--8",
            "utf__8",
            "utf--8-sig",
            "utf__8_sig",
            "-utf-8-sig",
            "utf8_ucs2",
            "utf8_ucs4",
            "utf8.ucs2",
            "utf8.ucs4",
            "cp65001",
        ] {
            let source = format!("# coding: {encoding}\npass\n");
            assert!(decode_source(source.as_bytes()).is_ok(), "{encoding}");
        }
        assert!(matches!(
            decode_source(b"# coding: utf.8\npass\n"),
            Err(SourceDecodeError::UnsupportedEncoding(encoding)) if encoding == "utf.8"
        ));
    }

    #[test]
    fn rejects_bom_for_normalized_utf8_sig_codec_aliases() {
        for encoding in ["utf--8-sig", "utf__8_sig", "-utf-8-sig"] {
            let source = format!("\u{feff}# coding: {encoding}\nvalue = 'é'\n");
            assert_eq!(
                decode_source(source.as_bytes()),
                Err(SourceDecodeError::Utf8BomConflict(encoding.to_string())),
            );
        }
    }

    #[test]
    fn accepts_cpython_legacy_encoding_aliases() {
        for encoding in [
            "8859",
            "cp819",
            "csisolatin1",
            "ibm819",
            "iso8859",
            "iso8859_1",
            "iso_8859_1",
            "iso_8859_1_1987",
            "iso_ir_100",
            "l1",
            "latin",
            "latin1",
            "latin_1",
            "iso_latin_1",
            "iso.8859.1",
            "iso.ir.100",
            "latin-1-unix",
            "iso-8859-1-unix",
            "iso-latin-1-unix",
        ] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::Latin1),
                "{encoding}"
            );
        }
        for encoding in ["euc_jp", "eucjp", "u_jis", "ujis", "u.jis"] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::EucJp),
                "{encoding}"
            );
        }
        for encoding in [
            "iso-8859-15",
            "iso8859_15",
            "iso_8859_15",
            "iso.8859.15",
            "latin9",
            "l9",
        ] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::Iso885915),
                "{encoding}"
            );
        }
        for encoding in ["cskoi8r", "koi8_r"] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::Koi8R),
                "{encoding}"
            );
        }
        for encoding in ["1251", "cp1251", "windows_1251", "windows.1251"] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::Windows1251),
                "{encoding}"
            );
        }
        for encoding in ["1252", "cp1252", "windows-1252", "windows.1252"] {
            assert_eq!(
                classify_source_encoding(encoding),
                Some(SourceEncoding::Windows1252),
                "{encoding}"
            );
        }
        for encoding in [
            "cp.1252",
            "latin.1",
            "iso-latin--1",
            "iso__latin_1",
            "euc.jp",
            "koi8.r",
            "cp.1251",
        ] {
            assert_eq!(classify_source_encoding(encoding), None, "{encoding}");
        }
    }

    #[test]
    fn recognizes_second_line_cookies_with_every_newline_form() {
        for newline in ["\n", "\r\n", "\r"] {
            let supported = format!("# first{newline}# coding: utf8{newline}pass{newline}");
            assert!(decode_source(supported.as_bytes()).is_ok(), "{newline:?}");

            let legacy = format!("# first{newline}# coding: latin-1{newline}pass{newline}");
            assert!(decode_source(legacy.as_bytes()).is_ok(), "{newline:?}");

            let unsupported = format!("# first{newline}# coding: koi8_u{newline}pass{newline}");
            assert_eq!(
                decode_source(unsupported.as_bytes()),
                Err(SourceDecodeError::UnsupportedEncoding("koi8_u".to_string())),
                "{newline:?}",
            );
        }
    }
}
