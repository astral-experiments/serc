use std::cell::Cell;
use std::rc::Rc;

use ruff_python_ast::visitor::Visitor;
use ruff_python_ast::{Comprehension, Expr};
use ruff_text_size::TextSize;
use rustc_hash::FxHashMap;

use crate::assembler::Assembler;
use crate::target::code_flags::{CO_FUTURE_ANNOTATIONS, CO_FUTURE_BARRY_AS_BDFL};
// The generated opcode module is the compiler lowering vocabulary; spelling out this nearly
// complete import would obscure which imports carry higher-level meaning.
#[allow(clippy::wildcard_imports)]
use crate::target::opcodes::*;

mod constant;
mod definition;
mod expression;
#[cfg(test)]
mod legacy_annotation_tests;
mod legacy_native_labels;
mod legacy_native_seed_bound;
mod native_profile;
pub(crate) mod optimization;
mod orchestration;
mod scope;
mod source;
mod statement;
mod symbol;
pub(crate) mod syntax;
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

pub(crate) use scope::MarshalNameLookup;

const SUPPORTED_FUTURE_FLAGS: u32 = CO_FUTURE_BARRY_AS_BDFL | CO_FUTURE_ANNOTATIONS;
const SHADOWED_SUPER_SENTINEL: &str = "\0shadowed super";
// CPython's flowgraph optimizer folds constant sequences only up to
// `_PY_STACK_USE_GUIDELINE` elements.
const CPYTHON_STACK_USE_GUIDELINE: usize = 30;

#[derive(Clone, Debug)]
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
pub(crate) struct Compiler<'source> {
    context: Rc<CompilationContext<'source>>,
    cached_source_lines: Cell<[Option<CachedSourceLine>; 2]>,
    unit: CodeUnit,
    output: OutputState,
    symbols: SymbolState,
    control: ControlFlowState<'source>,
    assembler: Assembler,
    native_label_plan: Option<Box<legacy_native_labels::NativeLabelPlan>>,
}

#[derive(Clone, Copy, Debug)]
struct CachedSourceLine {
    number: u32,
    start: TextSize,
    end: TextSize,
    is_last: bool,
}

impl Compiler<'_> {
    /// Python 3.14 introduced lazily evaluated annotation functions.
    fn has_deferred_annotations(&self) -> bool {
        matches!(
            self.context.python_version,
            crate::PythonVersion::Py314 | crate::PythonVersion::Py315
        )
    }

    /// Attach function payloads in the order consumed by the target runtime.
    fn emit_make_function(&mut self, flags: u32) -> Result<(), crate::CompileError> {
        use crate::target::operands::FunctionAttribute;

        if self.context.python_version == crate::PythonVersion::Py312 {
            return self.emit(
                MAKE_FUNCTION,
                flags,
                -i32::try_from(flags.count_ones()).unwrap(),
            );
        }
        self.emit(MAKE_FUNCTION, 0, 0)?;
        for attribute in [
            FunctionAttribute::CLOSURE,
            FunctionAttribute::ANNOTATIONS,
            FunctionAttribute::ANNOTATE,
            FunctionAttribute::KWDEFAULTS,
            FunctionAttribute::DEFAULTS,
        ] {
            if flags & attribute.argument() != 0 {
                self.emit_function_attribute(attribute)?;
            }
        }
        Ok(())
    }

    /// CPython 3.15 keeps an index slot alongside each synchronous iterator.
    fn iterator_extra_slots(&self) -> i32 {
        i32::from(self.context.python_version == crate::PythonVersion::Py315)
    }

    fn iterator_slots(&self) -> i32 {
        1 + self.iterator_extra_slots()
    }
}

impl CachedSourceLine {
    fn contains(self, offset: TextSize) -> bool {
        self.start <= offset && (offset < self.end || self.is_last && offset == self.end)
    }
}

/// Borrows list, set, and dict comprehension parts without visiting children.
/// Generator expressions retain their separate scope handling at each call site.
fn comprehension_parts(expression: &Expr) -> Option<(&[Comprehension], Option<&Expr>, &Expr)> {
    match expression {
        Expr::ListComp(comprehension) => {
            Some((&comprehension.generators, None, &comprehension.elt))
        }
        Expr::SetComp(comprehension) => Some((&comprehension.generators, None, &comprehension.elt)),
        Expr::DictComp(comprehension) => Some((
            &comprehension.generators,
            comprehension.key.as_deref(),
            &comprehension.value,
        )),
        _ => None,
    }
}

/// Annotation-only subscript targets evaluate slice bounds without constructing a slice.
fn walk_annotation_subscript<'ast>(visitor: &mut impl Visitor<'ast>, expression: &'ast Expr) {
    match expression {
        Expr::Slice(slice) => {
            for bound in [&slice.lower, &slice.upper, &slice.step]
                .into_iter()
                .flatten()
            {
                visitor.visit_expr(bound);
            }
        }
        Expr::Tuple(tuple) => {
            for element in &tuple.elts {
                walk_annotation_subscript(visitor, element);
            }
        }
        _ => visitor.visit_expr(expression),
    }
}
