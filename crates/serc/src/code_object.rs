use std::error::Error;
use std::fmt;

use rustc_hash::FxHashSet;

use crate::target::code_flags::{CO_VARARGS, CO_VARKEYWORDS};
use crate::target::local_kinds::CO_FAST_LOCAL;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeObjectValidationError {
    qualified_name: String,
    invariant: CodeObjectInvariant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CodeObjectInvariant {
    SignedFieldRange {
        field: CodeObjectField,
        value: u32,
    },
    BytecodeSize {
        bytes: usize,
    },
    BytecodeAlignment {
        bytes: usize,
    },
    LocalsShape {
        names: usize,
        kinds: usize,
    },
    PositionalOnlyArguments {
        positional_only: u32,
        positional: u32,
    },
    ParameterSlots {
        parameters: u64,
        local_slots: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodeObjectField {
    ArgumentCount,
    PositionalOnlyArgumentCount,
    KeywordOnlyArgumentCount,
    StackSize,
    Flags,
}

impl CodeObjectField {
    const fn name(self) -> &'static str {
        match self {
            Self::ArgumentCount => "argument count",
            Self::PositionalOnlyArgumentCount => "positional-only argument count",
            Self::KeywordOnlyArgumentCount => "keyword-only argument count",
            Self::StackSize => "stack size",
            Self::Flags => "flags",
        }
    }
}

impl CodeObjectValidationError {
    fn new(code: &CodeObject, invariant: CodeObjectInvariant) -> Self {
        Self {
            qualified_name: code.qualified_name.clone(),
            invariant,
        }
    }
}

impl fmt::Display for CodeObjectValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid code object `{}`: ", self.qualified_name)?;
        match self.invariant {
            CodeObjectInvariant::SignedFieldRange { field, value } => write!(
                formatter,
                "{} {value} exceeds CPython's signed 32-bit limit",
                field.name()
            ),
            CodeObjectInvariant::BytecodeSize { bytes } => write!(
                formatter,
                "bytecode length {bytes} exceeds CPython's signed 32-bit limit"
            ),
            CodeObjectInvariant::BytecodeAlignment { bytes } => {
                write!(
                    formatter,
                    "bytecode length {bytes} is not divisible by CPython's two-byte code unit"
                )
            }
            CodeObjectInvariant::LocalsShape { names, kinds } => write!(
                formatter,
                "locals metadata has {names} names but {kinds} kind entries"
            ),
            CodeObjectInvariant::PositionalOnlyArguments {
                positional_only,
                positional,
            } => write!(
                formatter,
                "declares {positional_only} positional-only arguments but only {positional} total positional arguments"
            ),
            CodeObjectInvariant::ParameterSlots {
                parameters,
                local_slots,
            } => write!(
                formatter,
                "declares {parameters} parameter slots but only {local_slots} local-variable slots"
            ),
        }
    }
}

impl Error for CodeObjectValidationError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LocalsIdentity {
    #[default]
    ByValue,
    Fresh,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MarshalHints {
    pub(crate) locals_identity: LocalsIdentity,
    pub(crate) explicitly_interned_strings: FxHashSet<String>,
    /// Constants created by iterable set folding before code-object string interning.
    pub(crate) iterable_frozenset_indices: FxHashSet<u32>,
}

impl MarshalHints {
    /// Compares the subset of marshal hints that participates in code-constant equality.
    pub(crate) fn equal_for_constant_pool(&self, other: &Self) -> bool {
        // CPython gives annotation-scope locals fresh identity, which distinguishes otherwise
        // equal code constants. Explicit intern hints only affect the later marshal traversal and
        // are intentionally omitted from constant-pool equality.
        self.locals_identity == other.locals_identity
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Constant {
    None,
    Bool(bool),
    Ellipsis,
    Int(u64),
    SignedInt(i64),
    BigInt {
        negative: bool,
        digits: Vec<u16>,
    },
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    String(String),
    SurrogateString(Vec<u8>),
    Bytes(Vec<u8>),
    Tuple(Vec<Self>),
    FrozenSet(Vec<Self>),
    Slice {
        lower: Box<Self>,
        upper: Box<Self>,
        step: Box<Self>,
    },
    Code(Box<CodeObject>),
}

#[derive(Clone, Debug)]
pub(crate) struct CodeObject {
    pub(crate) arg_count: u32,
    pub(crate) positional_only_arg_count: u32,
    pub(crate) keyword_only_arg_count: u32,
    pub(crate) stack_size: u32,
    pub(crate) flags: u32,
    pub(crate) bytecode: Vec<u8>,
    pub(crate) constants: Vec<Constant>,
    pub(crate) names: Vec<String>,
    pub(crate) locals: Vec<String>,
    pub(crate) local_kinds: Vec<u8>,
    pub(crate) filename: String,
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) first_line_number: u32,
    pub(crate) line_table: Vec<u8>,
    pub(crate) exception_table: Vec<u8>,
    pub(crate) marshal_hints: MarshalHints,
}

impl CodeObject {
    /// Checks the model invariants required before a code object can escape the compiler.
    pub(crate) fn validate(&self) -> Result<(), CodeObjectValidationError> {
        // Marshal writes these as 32-bit words that CPython reads into signed integer fields.
        for (field, value) in [
            (CodeObjectField::ArgumentCount, self.arg_count),
            (
                CodeObjectField::PositionalOnlyArgumentCount,
                self.positional_only_arg_count,
            ),
            (
                CodeObjectField::KeywordOnlyArgumentCount,
                self.keyword_only_arg_count,
            ),
            (CodeObjectField::StackSize, self.stack_size),
            (CodeObjectField::Flags, self.flags),
        ] {
            if value > i32::MAX as u32 {
                return Err(CodeObjectValidationError::new(
                    self,
                    CodeObjectInvariant::SignedFieldRange { field, value },
                ));
            }
        }
        if let Some(invariant) = bytecode_length_invariant(self.bytecode.len()) {
            return Err(CodeObjectValidationError::new(self, invariant));
        }
        if self.locals.len() != self.local_kinds.len() {
            return Err(CodeObjectValidationError::new(
                self,
                CodeObjectInvariant::LocalsShape {
                    names: self.locals.len(),
                    kinds: self.local_kinds.len(),
                },
            ));
        }
        if self.positional_only_arg_count > self.arg_count {
            return Err(CodeObjectValidationError::new(
                self,
                CodeObjectInvariant::PositionalOnlyArguments {
                    positional_only: self.positional_only_arg_count,
                    positional: self.arg_count,
                },
            ));
        }
        let parameter_count = u64::from(self.arg_count)
            + u64::from(self.keyword_only_arg_count)
            + u64::from(self.flags & CO_VARARGS != 0)
            + u64::from(self.flags & CO_VARKEYWORDS != 0);
        // CPython counts parameter capacity from local-variable entries, excluding cell-only and
        // free-variable entries in the parallel locals-plus arrays.
        let local_slots = self
            .local_kinds
            .iter()
            .filter(|kind| **kind & CO_FAST_LOCAL != 0)
            .count();
        if usize::try_from(parameter_count)
            .ok()
            .is_none_or(|count| count > local_slots)
        {
            return Err(CodeObjectValidationError::new(
                self,
                CodeObjectInvariant::ParameterSlots {
                    parameters: parameter_count,
                    local_slots,
                },
            ));
        }
        Ok(())
    }
}

fn bytecode_length_invariant(bytes: usize) -> Option<CodeObjectInvariant> {
    if bytes > i32::MAX as usize {
        Some(CodeObjectInvariant::BytecodeSize { bytes })
    } else if !bytes.is_multiple_of(2) {
        Some(CodeObjectInvariant::BytecodeAlignment { bytes })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CodeObject, CodeObjectField, CodeObjectInvariant, CodeObjectValidationError, Constant,
        MarshalHints, bytecode_length_invariant,
    };
    use crate::CompileError;
    use crate::target::local_kinds::CO_FAST_FREE;

    fn code_object() -> CodeObject {
        CodeObject {
            arg_count: 0,
            positional_only_arg_count: 0,
            keyword_only_arg_count: 0,
            stack_size: 1,
            flags: 0,
            bytecode: Vec::new(),
            constants: vec![Constant::None],
            names: Vec::new(),
            locals: Vec::new(),
            local_kinds: Vec::new(),
            filename: "module.py".to_string(),
            name: "<module>".to_string(),
            qualified_name: "<module>".to_string(),
            first_line_number: 1,
            line_table: Vec::new(),
            exception_table: Vec::new(),
            marshal_hints: MarshalHints::default(),
        }
    }

    #[test]
    fn accepts_a_valid_code_object() {
        code_object().validate().unwrap();
    }

    #[test]
    fn rejects_misaligned_bytecode_through_the_internal_error_path() {
        let mut code = code_object();
        code.bytecode.push(0);

        let error = code.validate().unwrap_err();
        assert_eq!(
            error,
            CodeObjectValidationError {
                qualified_name: "<module>".to_string(),
                invariant: CodeObjectInvariant::BytecodeAlignment { bytes: 1 },
            }
        );
        assert_eq!(
            CompileError::from(error),
            CompileError::Internal(
                "invalid code object `<module>`: bytecode length 1 is not divisible by CPython's two-byte code unit"
                    .to_string()
            )
        );
    }

    #[test]
    fn rejects_bytecode_larger_than_cpython_can_index() {
        let bytes = i32::MAX as usize + 1;
        assert_eq!(
            bytecode_length_invariant(bytes),
            Some(CodeObjectInvariant::BytecodeSize { bytes })
        );
    }

    #[test]
    fn rejects_code_fields_that_marshal_as_negative() {
        let value = i32::MAX as u32 + 1;
        let invalid = [
            (CodeObjectField::ArgumentCount, {
                let mut code = code_object();
                code.arg_count = value;
                code
            }),
            (CodeObjectField::PositionalOnlyArgumentCount, {
                let mut code = code_object();
                code.positional_only_arg_count = value;
                code
            }),
            (CodeObjectField::KeywordOnlyArgumentCount, {
                let mut code = code_object();
                code.keyword_only_arg_count = value;
                code
            }),
            (CodeObjectField::StackSize, {
                let mut code = code_object();
                code.stack_size = value;
                code
            }),
            (CodeObjectField::Flags, {
                let mut code = code_object();
                code.flags = value;
                code
            }),
        ];

        for (field, code) in invalid {
            assert!(matches!(
                code.validate().unwrap_err().invariant,
                CodeObjectInvariant::SignedFieldRange {
                    field: actual,
                    value: actual_value,
                } if actual == field && actual_value == value
            ));
        }
    }

    #[test]
    fn rejects_mismatched_locals_metadata() {
        let mut code = code_object();
        code.locals.push("value".to_string());

        assert!(matches!(
            code.validate().unwrap_err().invariant,
            CodeObjectInvariant::LocalsShape { names: 1, kinds: 0 }
        ));
    }

    #[test]
    fn rejects_positional_only_arguments_outside_the_positional_count() {
        let mut code = code_object();
        code.positional_only_arg_count = 1;

        assert!(matches!(
            code.validate().unwrap_err().invariant,
            CodeObjectInvariant::PositionalOnlyArguments {
                positional_only: 1,
                positional: 0
            }
        ));
    }

    #[test]
    fn rejects_parameters_without_local_slots() {
        let mut code = code_object();
        code.arg_count = 1;
        code.locals.push("captured".to_string());
        code.local_kinds.push(CO_FAST_FREE);

        assert!(matches!(
            code.validate().unwrap_err().invariant,
            CodeObjectInvariant::ParameterSlots {
                parameters: 1,
                local_slots: 0
            }
        ));
    }
}
