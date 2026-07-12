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

/// An opcode and its inline-cache width for the selected CPython target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Opcode {
    code: u8,
    caches: u8,
}

impl Opcode {
    /// This constructor is private so that generated target metadata is the sole numeric authority.
    const fn new(code: u8, caches: u8) -> Self {
        Self { code, caches }
    }

    pub(crate) const fn code(self) -> u8 {
        self.code
    }

    pub(crate) const fn caches(self) -> u8 {
        self.caches
    }
}

#[cfg(test)]
mod tests {
    use super::cpython_3_14_5::PREINTERNED_STRINGS;
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
            "500", "ascii", "darwin", "kwargs", "l1", "l2", "update", "win32",
        ] {
            assert!(
                is_preinterned_string(value),
                "missing startup string {value:?}"
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
