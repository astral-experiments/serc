//! Metadata for the exact CPython bytecode target.

pub(crate) mod cpython_3_12_14;
mod cpython_3_12_14_source_codecs;
pub(crate) mod cpython_3_13_15;
mod cpython_3_13_15_source_codecs;
mod cpython_3_14_5;
mod cpython_3_14_5_source_codecs;
pub(crate) mod cpython_3_15_0rc2;
mod cpython_3_15_0rc2_source_codecs;
mod legacy_opcodes;

use crate::PythonVersion;
use crate::source::SourceCodec;

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
                SourceCodec::EucJp => $module::EUC_JP,
                SourceCodec::Koi8R => $module::KOI8_R,
                SourceCodec::Windows1251 => $module::CP1251,
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

pub(crate) use cpython_3_12_14::operands as operands312;
pub(crate) use cpython_3_15_0rc2::operands as operands315;

/// Internal marker distinguishing generator-expression entry from a resume depth flag.
pub(crate) const GENEXPR_RESUME_ARGUMENT: u32 = 0x100;

pub(crate) use cpython_3_14_5::code_flags;
pub(crate) use cpython_3_14_5::local_kinds;
pub(crate) mod opcodes {
    pub(crate) use super::cpython_3_14_5::opcodes::*;
    pub(crate) use super::legacy_opcodes::{
        BEFORE_ASYNC_WITH, BEFORE_WITH, BINARY_SUBSCR, BUILD_CONST_KEY_MAP, ENTER_EXECUTOR,
        FORMAT_VALUE, KW_NAMES, LOAD_ASSERTION_ERROR, LOAD_CLOSURE, RETURN_CONST,
    };
}
pub(crate) use cpython_3_14_5::operands;
pub(crate) use cpython_3_14_5::{
    TARGET_COMMIT, TARGET_IMPLEMENTATION, TARGET_MAGIC_NUMBER, TARGET_TAG, TARGET_VERSION,
    is_conditional_jump, is_preinterned_string, is_unconditional_jump, stack_counts,
};

/// Recognizes scope exits across the semantic IR, including legacy constant returns.
pub(crate) const fn is_scope_exit(opcode: Opcode) -> bool {
    cpython_3_14_5::is_scope_exit(opcode) || matches!(opcode, opcodes::RETURN_CONST)
}

/// An opcode for the selected CPython target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Opcode {
    code: u8,
}

/// Cache widths indexed by the semantic IR opcode for one exact target release.
#[derive(Clone, Copy)]
pub(crate) struct OpcodeCacheWidths(&'static [u8; 256]);

impl OpcodeCacheWidths {
    #[inline]
    pub(crate) const fn for_version(version: PythonVersion) -> Self {
        let index = match version {
            PythonVersion::Py312 => 0,
            PythonVersion::Py313 => 1,
            PythonVersion::Py314 => 2,
            PythonVersion::Py315 => 3,
        };
        Self(&OPCODE_CACHE_WIDTHS[index])
    }

    #[inline]
    pub(crate) const fn get(self, opcode: Opcode) -> u8 {
        self.0[opcode.code() as usize]
    }
}

static OPCODE_CACHE_WIDTHS: [[u8; 256]; 4] = {
    let mut widths = [[0; 256]; 4];
    let mut code = 0_u8;
    loop {
        let opcode = Opcode::new(code);
        let index = code as usize;
        widths[0][index] = cpython_3_12_14::caches(opcode);
        widths[1][index] = cpython_3_13_15::caches(opcode);
        widths[2][index] = opcode.caches();
        widths[3][index] = cpython_3_15_0rc2::caches(opcode);
        if code == u8::MAX {
            break;
        }
        code += 1;
    }
    widths
};

impl Opcode {
    /// Read mnemonics from the same generated metadata that defines the IR opcodes.
    #[cfg(test)]
    pub(crate) fn name(self) -> &'static str {
        let code = self.code();
        for source in [
            include_str!("cpython_3_14_5.rs"),
            include_str!("legacy_opcodes.rs"),
        ] {
            for line in source.lines() {
                if let Some(declaration) = line.trim().strip_prefix("pub(crate) const ")
                    && let Some((name, value)) = declaration.split_once(": Opcode = Opcode::new(")
                    && value
                        .strip_suffix(");")
                        .and_then(|value| value.parse::<u8>().ok())
                        == Some(code)
                {
                    return name;
                }
            }
        }
        panic!("missing generated mnemonic for opcode {code}");
    }

    /// This constructor is private so that generated target metadata is the sole numeric authority.
    const fn new(code: u8) -> Self {
        Self { code }
    }

    pub(crate) const fn code(self) -> u8 {
        self.code
    }

    /// Returns the wire opcode generated for the selected CPython release.
    pub(crate) const fn code_for(self, version: PythonVersion) -> u8 {
        match version {
            PythonVersion::Py312 => cpython_3_12_14::code(self),
            PythonVersion::Py313 => cpython_3_13_15::code(self),
            PythonVersion::Py314 => {
                assert!(self.code() <= 128, "legacy opcode used for CPython 3.14");
                self.code()
            }
            PythonVersion::Py315 => cpython_3_15_0rc2::code(self),
        }
    }

    /// Returns the selected release's inline-cache width.
    #[inline]
    pub(crate) const fn caches_for(self, version: PythonVersion) -> u8 {
        OpcodeCacheWidths::for_version(version).get(self)
    }

    pub(crate) const fn has_jump_for(self, version: PythonVersion) -> bool {
        match version {
            PythonVersion::Py312 => cpython_3_12_14::has_jump(self),
            PythonVersion::Py313 => cpython_3_13_15::has_jump(self),
            PythonVersion::Py314 => self.code() <= 128 && self.has_jump(),
            PythonVersion::Py315 => cpython_3_15_0rc2::has_jump(self),
        }
    }
}

pub(crate) const fn is_scope_exit_for(version: PythonVersion, opcode: Opcode) -> bool {
    match version {
        PythonVersion::Py312 => cpython_3_12_14::is_scope_exit(opcode),
        PythonVersion::Py313 => cpython_3_13_15::is_scope_exit(opcode),
        PythonVersion::Py314 => cpython_3_14_5::is_scope_exit(opcode),
        PythonVersion::Py315 => cpython_3_15_0rc2::is_scope_exit(opcode),
    }
}

pub(crate) const fn is_conditional_jump_for(version: PythonVersion, opcode: Opcode) -> bool {
    match version {
        PythonVersion::Py312 => cpython_3_12_14::is_conditional_jump(opcode),
        PythonVersion::Py313 => cpython_3_13_15::is_conditional_jump(opcode),
        PythonVersion::Py314 => cpython_3_14_5::is_conditional_jump(opcode),
        PythonVersion::Py315 => cpython_3_15_0rc2::is_conditional_jump(opcode),
    }
}

/// Returns stack input and output counts for the selected bytecode target.
pub(crate) fn stack_counts_for(
    version: PythonVersion,
    opcode: Opcode,
    argument: u32,
) -> (usize, usize) {
    match version {
        PythonVersion::Py312 => cpython_3_12_14::stack_counts(opcode, argument),
        PythonVersion::Py313 => cpython_3_13_15::stack_counts(opcode, argument),
        PythonVersion::Py314 => stack_counts(opcode, argument),
        PythonVersion::Py315 => cpython_3_15_0rc2::stack_counts(opcode, argument),
    }
}

/// Checks the selected release's static, startup and compiler interned strings.
pub(crate) fn is_preinterned_string_for(version: PythonVersion, value: &str) -> bool {
    match version {
        PythonVersion::Py312 => cpython_3_12_14::is_preinterned_string(value),
        PythonVersion::Py313 => cpython_3_13_15::is_preinterned_string(value),
        PythonVersion::Py314 => is_preinterned_string(value),
        PythonVersion::Py315 => cpython_3_15_0rc2::is_preinterned_string(value),
    }
}

/// Returns inclusive bounds of the target interpreter's cached integer singletons.
pub(crate) const fn small_int_bounds_for(version: PythonVersion) -> (i64, i64) {
    match version {
        PythonVersion::Py312 => (
            cpython_3_12_14::SMALL_INT_MIN,
            cpython_3_12_14::SMALL_INT_MAX,
        ),
        PythonVersion::Py313 => (
            cpython_3_13_15::SMALL_INT_MIN,
            cpython_3_13_15::SMALL_INT_MAX,
        ),
        // CPython 3.14.5 Include/internal/pycore_runtime_structs.h.
        PythonVersion::Py314 => (-5, 256),
        PythonVersion::Py315 => (
            cpython_3_15_0rc2::SMALL_INT_MIN,
            cpython_3_15_0rc2::SMALL_INT_MAX,
        ),
    }
}

/// Returns the SEND-to-END_SEND offset for the release's yield-from sequence.
pub(crate) const fn end_send_offset_for(version: PythonVersion) -> u32 {
    match version {
        // Legacy END_ASYNC_FOR has no SEND-relative jump operand.
        PythonVersion::Py312 | PythonVersion::Py313 => 0,
        // CPython 3.14.5 Python/assemble.c: END_SEND_OFFSET.
        PythonVersion::Py314 => 5,
        PythonVersion::Py315 => cpython_3_15_0rc2::END_SEND_OFFSET,
    }
}

#[cfg(test)]
mod tests {
    use super::cpython_3_14_5::{COMPILER_INTERNED_STRINGS, PREINTERNED_STRINGS};
    use super::is_preinterned_string;

    #[test]
    fn cache_width_tables_match_every_generated_semantic_opcode() {
        use super::{
            Opcode, OpcodeCacheWidths, cpython_3_12_14, cpython_3_13_15, cpython_3_15_0rc2,
        };
        use crate::PythonVersion::{Py312, Py313, Py314, Py315};

        type CacheWidth = fn(Opcode) -> u8;
        let generated: [(crate::PythonVersion, CacheWidth); 4] = [
            (Py312, cpython_3_12_14::caches),
            (Py313, cpython_3_13_15::caches),
            (Py314, Opcode::caches),
            (Py315, cpython_3_15_0rc2::caches),
        ];
        for (version, expected) in generated {
            let table = OpcodeCacheWidths::for_version(version);
            for code in u8::MIN..=u8::MAX {
                let opcode = Opcode::new(code);
                assert_eq!(table.get(opcode), expected(opcode));
                assert_eq!(opcode.caches_for(version), expected(opcode));
            }
        }
    }

    #[test]
    fn legacy_targets_keep_semantic_identity_and_exact_stack_shapes() {
        use super::{is_scope_exit, opcodes, stack_counts_for};
        use crate::PythonVersion::{Py312, Py313, Py314};

        assert_eq!(opcodes::LOAD_CONST.code(), 82);
        assert_eq!(opcodes::RETURN_CONST.code_for(Py312), 121);
        assert_eq!(opcodes::RETURN_CONST.code_for(Py313), 103);
        assert_eq!(opcodes::KW_NAMES.code_for(Py312), 172);
        assert_eq!(opcodes::BEFORE_WITH.code_for(Py312), 53);
        assert_eq!(opcodes::BEFORE_WITH.code_for(Py313), 2);
        assert!(is_scope_exit(opcodes::RETURN_CONST));
        assert_eq!(stack_counts_for(Py312, opcodes::END_FOR, 0), (2, 0));
        assert_eq!(stack_counts_for(Py313, opcodes::END_FOR, 0), (1, 0));
        assert_eq!(stack_counts_for(Py312, opcodes::MAKE_FUNCTION, 15), (5, 1));
        assert_eq!(stack_counts_for(Py312, opcodes::FORMAT_VALUE, 4), (2, 1));
        assert_eq!(stack_counts_for(Py312, opcodes::KW_NAMES, 0), (0, 0));
        assert_eq!(opcodes::POP_JUMP_IF_FALSE.caches_for(Py312), 0);
        assert_eq!(opcodes::POP_JUMP_IF_FALSE.caches_for(Py313), 1);
        assert!(!opcodes::END_ASYNC_FOR.has_jump_for(Py312));
        assert!(!opcodes::END_ASYNC_FOR.has_jump_for(Py313));
        assert!(opcodes::END_ASYNC_FOR.has_jump_for(Py314));
    }

    #[test]
    fn legacy_operands_preserve_comparison_and_format_layouts() {
        use super::cpython_3_13_15::operands as operands313;
        use super::operands312;

        assert_eq!(operands312::ComparisonOperation::EQ.argument(), 40);
        assert_eq!(operands313::ComparisonOperation::EQ.argument(), 72);
        assert_eq!(operands313::ComparisonOperation::EQ.boolean_argument(), 88);
        assert_eq!(operands312::FormatValue::REPR.argument(), 2);
        assert_eq!(operands312::FormatValue::HAVE_SPEC.argument(), 4);
        assert_eq!(operands313::ResumeLocation::AFTER_AWAIT.at_depth(true), 7);
    }

    #[test]
    fn legacy_intern_lookups_include_their_independent_profiles() {
        use super::{cpython_3_12_14, cpython_3_13_15, is_preinterned_string_for};
        use crate::PythonVersion::{Py312, Py313};

        for (version, static_strings, compiler_strings, startup_strings) in [
            (
                Py312,
                cpython_3_12_14::PREINTERNED_STRINGS,
                cpython_3_12_14::COMPILER_INTERNED_STRINGS,
                cpython_3_12_14::STARTUP_INTERNED_STRINGS,
            ),
            (
                Py313,
                cpython_3_13_15::PREINTERNED_STRINGS,
                cpython_3_13_15::COMPILER_INTERNED_STRINGS,
                cpython_3_13_15::STARTUP_INTERNED_STRINGS,
            ),
        ] {
            assert!(static_strings.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(startup_strings.windows(2).all(|pair| pair[0] < pair[1]));
            for value in static_strings.iter().chain(compiler_strings) {
                assert!(
                    is_preinterned_string_for(version, value),
                    "{version}: {value:?}"
                );
            }
            for value in startup_strings {
                assert_eq!(
                    is_preinterned_string_for(version, value),
                    cfg!(target_os = "linux")
                );
            }
        }
    }

    #[test]
    fn selected_target_preserves_semantic_opcode_identity() {
        use super::{Opcode, cpython_3_15_0rc2, opcodes, stack_counts_for};
        use crate::PythonVersion::{Py314, Py315};

        assert_eq!(opcodes::GET_ITER.code_for(Py314), 16);
        assert_eq!(opcodes::GET_ITER.code_for(Py315), 70);
        assert_eq!(opcodes::GET_YIELD_FROM_ITER.code_for(Py315), 70);
        assert_eq!(opcodes::RESUME.caches_for(Py314), 0);
        assert_eq!(opcodes::RESUME.caches_for(Py315), 1);
        assert_eq!(opcodes::GET_ITER.caches_for(Py314), 0);
        assert_eq!(opcodes::GET_ITER.caches_for(Py315), 1);
        assert_eq!(stack_counts_for(Py314, opcodes::GET_ITER, 0), (1, 1));
        assert_eq!(stack_counts_for(Py315, opcodes::GET_ITER, 0), (1, 2));
        assert_eq!(stack_counts_for(Py314, opcodes::END_SEND, 0), (2, 1));
        assert_eq!(stack_counts_for(Py315, opcodes::END_SEND, 0), (3, 1));
        assert_eq!(super::end_send_offset_for(Py314), 5);
        assert_eq!(super::end_send_offset_for(Py315), 6);
        assert_eq!(super::small_int_bounds_for(Py314), (-5, 256));
        assert_eq!(super::small_int_bounds_for(Py315), (-5, 1024));
        assert_eq!(
            cpython_3_15_0rc2::TARGET_MAGIC_NUMBER,
            [0x52, 0x0e, 0x0d, 0x0a]
        );
        // Every compiler opcode keeps the original IR identity while obtaining
        // its encoding and stack behavior from the selected target table.
        for code in (0..=120).chain([128]) {
            let opcode = Opcode::new(code);
            let _ = opcode.code_for(Py315);
            let _ = opcode.caches_for(Py315);
        }
    }

    #[test]
    fn python315_interned_lookup_includes_generated_members() {
        use super::cpython_3_15_0rc2::{
            COMPILER_INTERNED_STRINGS, PREINTERNED_STRINGS, is_preinterned_string,
        };
        assert!(
            PREINTERNED_STRINGS
                .windows(2)
                .all(|values| values[0] < values[1])
        );
        for value in PREINTERNED_STRINGS.iter().chain(COMPILER_INTERNED_STRINGS) {
            assert!(
                is_preinterned_string(value),
                "missing generated string {value:?}"
            );
        }
        for value in super::cpython_3_15_0rc2::STARTUP_INTERNED_STRINGS {
            assert_eq!(is_preinterned_string(value), cfg!(target_os = "linux"));
        }
        assert!(is_preinterned_string("__lazy_import__"));
        assert!(!is_preinterned_string("__serc_not_interned__"));
    }

    #[test]
    fn neutral_bootstrap_lookup_covers_source_derived_families() {
        use super::cpython_3_14_5::BOOTSTRAP_INTERNED_STRINGS;

        assert!(
            BOOTSTRAP_INTERNED_STRINGS
                .windows(2)
                .all(|values| values[0] < values[1])
        );
        for value in BOOTSTRAP_INTERNED_STRINGS {
            assert!(
                is_preinterned_string(value) || !cfg!(target_os = "linux"),
                "missing source-derived bootstrap string {value:?}",
            );
        }
        for value in [
            "location",
            "_WeakValueDictionary.__init__.<locals>.KeyedRef",
            "FileFinder.path_hook.<locals>.path_hook_for_FileFinder",
            "normalize_encoding",
            "us",
        ] {
            assert!(BOOTSTRAP_INTERNED_STRINGS.binary_search(&value).is_ok());
            assert_eq!(is_preinterned_string(value), cfg!(target_os = "linux"));
        }
        for value in ["auto", "not", "char", "function"] {
            assert!(!is_preinterned_string(value));
        }
    }

    #[test]
    fn preinterned_lookup_preserves_exact_membership() {
        assert!(
            PREINTERNED_STRINGS
                .windows(2)
                .all(|values| values[0] < values[1])
        );
        for value in PREINTERNED_STRINGS {
            assert!(
                is_preinterned_string(value),
                "missing preinterned string {value:?}"
            );
        }
        for value in [
            "500",
            "MISSING",
            "None",
            "ascii",
            "cache",
            "cached",
            "candidate",
            "darwin",
            "date",
            "error",
            "eval",
            "kwargs",
            "l1",
            "l2",
            "link",
            "map",
            "min",
            "missing",
            "nan",
            "new",
            "old_value",
            "other",
            "output",
            "package",
            "python",
            "quiet",
            "remove",
            "root",
            "run",
            "sum",
            "system",
            "test",
            "timestamp",
            "u7",
            "unknown",
            "update",
            "user",
            "var",
            "warn",
            "win32",
        ] {
            assert!(
                is_preinterned_string(value),
                "missing startup string {value:?}"
            );
        }
        assert_eq!(is_preinterned_string("pause"), cfg!(unix));
        for value in COMPILER_INTERNED_STRINGS {
            assert!(
                is_preinterned_string(value),
                "missing compiler-initialized string {value:?}"
            );
        }
        for value in [
            "a",
            "5",
            "50",
            "5000",
            "al",
            "alpha",
            "ascii_",
            "identifier",
            "sort0",
            "left_extra",
            "desc",
            "failed",
            "full",
            "high",
            "partial",
            "__not_preinterned__",
            "méthode",
        ] {
            assert!(
                !is_preinterned_string(value),
                "unexpected preinterned string {value:?}"
            );
        }
    }

    #[test]
    fn preinterned_lookup_rejects_nonmembers_with_shared_prefixes() {
        for value in [
            "BuiltinImporter\0",
            "find_spec_suffix",
            "__name__\0",
            "utf_8_suffix",
            "\0",
            "méthode",
            "☃",
        ] {
            assert!(!is_preinterned_string(value), "unexpected member {value:?}");
        }
    }
}
