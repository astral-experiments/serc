use std::borrow::Cow;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;

use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use ruff_python_ast::{
    BoolOp, CmpOp, ConversionFlag, Expr, ExprBinOp, ExprContext, FStringPart, Identifier,
    InterpolatedStringElement, Keyword, Number, Operator, Pattern, Singleton, Stmt, StmtClassDef,
    StmtFunctionDef, Suite, TypeParam, UnaryOp,
};
use ruff_python_codegen::{Generator, Indentation, Mode as CodegenMode};
use ruff_text_size::{Ranged, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

type HashMap<K, V> = FxHashMap<K, V>;
type HashSet<T> = FxHashSet<T>;

use crate::CompileError;
use crate::assembler::{AssembledCode, Assembler, Label, SourceLocation};
use crate::code_object::{CodeObject, Constant};
use crate::target::code_flags::{
    CO_ASYNC_GENERATOR, CO_COROUTINE, CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL, CO_GENERATOR,
    CO_HAS_DOCSTRING, CO_METHOD, CO_NESTED, CO_NEWLOCALS, CO_OPTIMIZED, CO_VARARGS, CO_VARKEYWORDS,
};
use crate::target::local_kinds::{
    CO_FAST_ARG_KW, CO_FAST_ARG_POS, CO_FAST_ARG_VAR, CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN,
    CO_FAST_LOCAL,
};
// The generated opcode module is the compiler lowering vocabulary; spelling out this nearly
// complete import would obscure which imports carry higher-level meaning.
#[allow(clippy::wildcard_imports)]
use crate::target::opcodes::*;
use crate::target::operands::{
    BinaryIntrinsic, BinaryOperation, CommonConstant, ComparisonOperation, Conversion,
    FunctionAttribute, ResumeLocation, SpecialMethod, UnaryIntrinsic,
};
use crate::target::{Opcode, is_unconditional_jump};

mod constant;
mod definition;
mod expression;
mod orchestration;
mod scope;
mod source;
mod statement;
mod symbol;
#[cfg(test)]
mod tests;

// Compiler submodules intentionally share the parent-owned state and lowering vocabulary.
#[allow(clippy::wildcard_imports)]
use self::{
    constant::*, definition::*, expression::*, orchestration::CodeUnit, scope::*, source::*,
    statement::*, symbol::*,
};

// CPython initially emits owned local loads, then strength-reduces safe loads
// to borrowed references after the control-flow graph has reached its final
// shape. The assembler mirrors that dataflow pass.
const SUPPORTED_FUTURE_FLAGS: u32 = CO_FUTURE_BARRY_AS_BDFL | CO_FUTURE_ANNOTATIONS;
const SHADOWED_SUPER_SENTINEL: &str = "\0shadowed super";

#[derive(Debug)]
enum Scope {
    Module,
    Class {
        globals: NameSet,
        nonlocals: NameSet,
        free_indices: FxHashMap<NameId, u32>,
        bound_names: NameSet,
    },
    Function {
        indices: FxHashMap<NameId, u32>,
        free_indices: FxHashMap<NameId, u32>,
        cells: NameSet,
        globals: NameSet,
    },
}

#[derive(Clone, Copy, Debug)]
struct LoopContext {
    continue_label: Label,
    break_label: Label,
    iterator_cleanup: IteratorCleanup,
    with_depth: usize,
    finally_end_depth: usize,
    exception_region_depth: usize,
    break_returns: bool,
    preserve_break_exit: bool,
}

#[derive(Debug)]
struct ExceptionHandlerContext {
    name: Option<String>,
    loop_depth: usize,
}

#[derive(Debug)]
struct ReturnFinallyContext {
    body: Vec<Stmt>,
    loop_depth: usize,
    handler: Label,
    depth: u32,
}

#[derive(Clone, Copy, Debug)]
struct WithExitContext {
    location: SourceLocation,
    is_async: bool,
}

#[derive(Debug)]
struct MatchContext {
    stores: Vec<String>,
    fail_pop: Vec<Label>,
    on_top: usize,
}

type RegionExclusions = Vec<(Label, Label)>;
type RegionExclusionStack = Vec<RegionExclusions>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IteratorCleanup {
    None,
    Sync,
    Async,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollectionKind {
    List,
    Tuple,
    Set,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DefinitionDecorators {
    Compile,
    AlreadyCompiled,
}

#[derive(Clone, Copy, Debug)]
struct CompiledFunctionDefaults {
    positional: bool,
    keyword: bool,
}

impl CompiledFunctionDefaults {
    fn count(self) -> usize {
        usize::from(self.positional) + usize::from(self.keyword)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpecializedLowering {
    Applied,
    NotApplicable,
}

#[derive(Clone, Copy, Debug)]
enum SuperAttributeKind {
    Attribute,
    Method,
}

impl SuperAttributeKind {
    const fn method_flag(self) -> u32 {
        match self {
            Self::Attribute => 0,
            Self::Method => 1,
        }
    }

    const fn stack_effect(self) -> i32 {
        match self {
            Self::Attribute => -2,
            Self::Method => -1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ComparisonMode {
    Value,
    Boolean,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoopBackedge {
    Emitted,
    Required,
}

#[derive(Clone, Copy, Debug)]
struct TryCompileOptions {
    terminal: bool,
    emit_statement_nop: bool,
    exclude_terminal_body_not_taken: bool,
}

impl TryCompileOptions {
    const STANDARD: Self = Self {
        terminal: false,
        emit_statement_nop: true,
        exclude_terminal_body_not_taken: false,
    };
    const TERMINAL: Self = Self {
        terminal: true,
        ..Self::STANDARD
    };
    const LOOP_TAIL: Self = Self {
        exclude_terminal_body_not_taken: true,
        ..Self::STANDARD
    };
    const NESTED_HANDLER: Self = Self {
        terminal: false,
        emit_statement_nop: false,
        exclude_terminal_body_not_taken: false,
    };
}

#[derive(Debug)]
pub(crate) struct Compiler {
    context: Arc<CompilationContext>,
    cached_source_line: Cell<Option<CachedSourceLine>>,
    unit: CodeUnit,
    output: OutputState,
    symbols: SymbolState,
    control: ControlFlowState,
    assembler: Assembler,
}

#[derive(Clone, Copy, Debug)]
struct CachedSourceLine {
    number: u32,
    start: TextSize,
    end: TextSize,
    is_last: bool,
}

impl CachedSourceLine {
    fn contains(self, offset: TextSize) -> bool {
        self.start <= offset && (offset < self.end || self.is_last && offset == self.end)
    }
}
