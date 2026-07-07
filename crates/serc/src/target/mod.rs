//! Metadata for the exact CPython bytecode target.

mod cpython_3_14_5;

pub(crate) use cpython_3_14_5::code_flags;
pub(crate) use cpython_3_14_5::local_kinds;
pub(crate) use cpython_3_14_5::opcodes;
pub(crate) use cpython_3_14_5::operands;
pub(crate) use cpython_3_14_5::{
    TARGET_COMMIT, TARGET_IMPLEMENTATION, TARGET_MAGIC_NUMBER, TARGET_TAG, TARGET_VERSION,
    is_conditional_jump, is_scope_exit, is_unconditional_jump, num_popped, num_pushed,
    stack_effect,
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
    use super::opcodes::{BUILD_MAP, BUILD_TUPLE, LOAD_ATTR, RERAISE, UNPACK_EX};
    use super::{num_popped, num_pushed, stack_effect};

    #[test]
    fn generated_stack_effect_matches_component_counts() {
        for (opcode, argument) in [
            (BUILD_TUPLE, 3),
            (BUILD_MAP, 4),
            (LOAD_ATTR, 1),
            (RERAISE, 2),
            (UNPACK_EX, 0x0203),
        ] {
            let component_effect = i64::try_from(num_pushed(opcode, argument)).unwrap()
                - i64::try_from(num_popped(opcode, argument)).unwrap();
            assert_eq!(stack_effect(opcode, argument), component_effect);
        }
    }
}
