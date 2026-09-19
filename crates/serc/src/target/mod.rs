//! Metadata for the exact CPython bytecode target.

mod cpython_3_14_5;

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
}

#[cfg(test)]
mod tests {
    use super::cpython_3_14_5::{COMPILER_INTERNED_STRINGS, PREINTERNED_STRINGS};
    use super::is_preinterned_string;

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
