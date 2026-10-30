//! Compiler-specific source codecs and semantic markers.
mod cpython_3_12_14_source_codecs;
mod cpython_3_13_15_source_codecs;
mod cpython_3_14_5_source_codecs;
mod cpython_3_15_0rc2_source_codecs;

use crate::PythonVersion;
use crate::source::SourceCodec;
pub(crate) use serc_bytecode::target::*;

/// Source-derived retained strings from a standard codec imported while decoding.
pub(crate) fn source_codec_strings(
    version: PythonVersion,
    codec: SourceCodec,
) -> &'static [&'static str] {
    macro_rules! strings {
        ($module:ident) => {
            match codec {
                SourceCodec::Ascii => $module::ASCII,
                SourceCodec::Latin1 => $module::LATIN_1,
                SourceCodec::Iso885915 => $module::ISO8859_15,
                SourceCodec::Big5 => $module::BIG5,
                SourceCodec::EucJp => $module::EUC_JP,
                SourceCodec::Koi8R => $module::KOI8_R,
                SourceCodec::Windows1251 => $module::CP1251,
                SourceCodec::Windows1252 => $module::CP1252,
            }
        };
    }
    match version {
        PythonVersion::Py312 => strings!(cpython_3_12_14_source_codecs),
        PythonVersion::Py313 => strings!(cpython_3_13_15_source_codecs),
        PythonVersion::Py314 => strings!(cpython_3_14_5_source_codecs),
        PythonVersion::Py315 => strings!(cpython_3_15_0rc2_source_codecs),
    }
}

/// Internal marker distinguishing generator-expression entry from a resume depth flag.
pub(crate) const GENEXPR_RESUME_ARGUMENT: u32 = 0x100;
