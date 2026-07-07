use std::cell::Cell;
use std::sync::Arc;

use ruff_text_size::TextSize;
use rustc_hash::{FxHashMap, FxHashSet};

type HashMap<K, V> = FxHashMap<K, V>;
type HashSet<T> = FxHashSet<T>;

use crate::assembler::Assembler;
use crate::target::code_flags::{CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL};
// The generated opcode module is the compiler lowering vocabulary; spelling out this nearly
// complete import would obscure which imports carry higher-level meaning.
#[allow(clippy::wildcard_imports)]
use crate::target::opcodes::*;

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

use self::{
    constant::OutputState,
    orchestration::CodeUnit,
    scope::{NameId, NameSet},
    source::CompilationContext,
    statement::ControlFlowState,
    symbol::SymbolState,
};

// CPython initially emits owned local loads, then strength-reduces safe loads
// to borrowed references after the control-flow graph has reached its final
// shape. The assembler mirrors that dataflow pass.
const SUPPORTED_FUTURE_FLAGS: u32 = CO_FUTURE_BARRY_AS_BDFL | CO_FUTURE_ANNOTATIONS;
const SHADOWED_SUPER_SENTINEL: &str = "\0shadowed super";

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
    pub(crate) annotation_thunk: bool,
    pub(crate) interned_constant_strings: HashSet<String>,
}

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
