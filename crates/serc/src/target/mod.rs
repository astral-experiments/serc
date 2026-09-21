//! Metadata for the exact CPython bytecode target.

mod cpython_3_14_5;
pub(crate) mod cpython_3_15_0rc2;

use crate::PythonVersion;

pub(crate) use cpython_3_15_0rc2::operands as operands315;

/// Internal marker distinguishing generator-expression entry from a resume depth flag.
pub(crate) const GENEXPR_RESUME_ARGUMENT: u32 = 0x100;

pub(crate) use cpython_3_14_5::code_flags;
pub(crate) use cpython_3_14_5::local_kinds;
pub(crate) use cpython_3_14_5::opcodes;
pub(crate) use cpython_3_14_5::operands;
pub(crate) use cpython_3_14_5::{
    TARGET_COMMIT, TARGET_IMPLEMENTATION, TARGET_MAGIC_NUMBER, TARGET_TAG, TARGET_VERSION,
    is_conditional_jump, is_preinterned_string, is_scope_exit, is_unconditional_jump, stack_counts,
};

/// An opcode for the selected CPython target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Opcode {
    code: u8,
}

impl Opcode {
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
            PythonVersion::Py314 => self.code(),
            PythonVersion::Py315 => cpython_3_15_0rc2::code(self),
        }
    }

    /// Returns the selected release's inline-cache width.
    pub(crate) const fn caches_for(self, version: PythonVersion) -> u8 {
        match version {
            PythonVersion::Py314 => self.caches(),
            PythonVersion::Py315 => cpython_3_15_0rc2::caches(self),
        }
    }
}

/// Returns stack input and output counts for the selected bytecode target.
pub(crate) fn stack_counts_for(
    version: PythonVersion,
    opcode: Opcode,
    argument: u32,
) -> (usize, usize) {
    match version {
        PythonVersion::Py314 => stack_counts(opcode, argument),
        PythonVersion::Py315 => cpython_3_15_0rc2::stack_counts(opcode, argument),
    }
}

/// Checks the selected release's static, startup and compiler interned strings.
pub(crate) fn is_preinterned_string_for(version: PythonVersion, value: &str) -> bool {
    match version {
        PythonVersion::Py314 => is_preinterned_string(value),
        PythonVersion::Py315 => cpython_3_15_0rc2::is_preinterned_string(value),
    }
}

/// Returns inclusive bounds of the target interpreter's cached integer singletons.
pub(crate) const fn small_int_bounds_for(version: PythonVersion) -> (i64, i64) {
    match version {
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
}
