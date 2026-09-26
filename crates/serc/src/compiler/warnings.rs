//! Native compiler warning predicates, separate from warning emission and ordering.
//!
//! Call these at the corresponding lowering operation. Repeated finalizer copies and
//! warning suppression belong to the caller, not to these classifiers. The predicates
//! model optimization level zero, which is the compiler's supported configuration.

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, CmpOp, Expr, ExprContext, Operator, Stmt, UnaryOp};
use ruff_text_size::{Ranged, TextRange};

use crate::PythonVersion;
use crate::code_object::Constant;

use super::constant::{
    fold_constant_with_literals, literal_constant, optimized_percent_format,
    optimized_percent_format_arguments,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WarningKind {
    IdentityComparison,
    AssertionAlwaysTrue,
    NonCallable,
    NonSubscriptable,
    InvalidIndex,
    FinallyReturn,
    FinallyBreak,
    FinallyContinue,
}

impl WarningKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::IdentityComparison => "identity_comparison",
            Self::AssertionAlwaysTrue => "assertion_always_true",
            Self::NonCallable => "non_callable",
            Self::NonSubscriptable => "non_subscriptable",
            Self::InvalidIndex => "invalid_index",
            Self::FinallyReturn => "finally_return",
            Self::FinallyBreak => "finally_break",
            Self::FinallyContinue => "finally_continue",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Warning {
    pub(super) kind: WarningKind,
    pub(super) range: TextRange,
    pub(super) message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeType {
    None,
    Bool,
    Ellipsis,
    Int,
    Float,
    Complex,
    Str,
    Bytes,
    Tuple,
    List,
    Dict,
    Set,
    FrozenSet,
    Generator,
    Function,
    Template,
    Slice,
    Code,
}

impl NativeType {
    const fn name(self) -> &'static str {
        match self {
            Self::None => "NoneType",
            Self::Bool => "bool",
            Self::Ellipsis => "ellipsis",
            Self::Int => "int",
            Self::Float => "float",
            Self::Complex => "complex",
            Self::Str => "str",
            Self::Bytes => "bytes",
            Self::Tuple => "tuple",
            Self::List => "list",
            Self::Dict => "dict",
            Self::Set => "set",
            Self::FrozenSet => "frozenset",
            Self::Generator => "generator",
            Self::Function => "function",
            Self::Template => "string.templatelib.Template",
            Self::Slice => "slice",
            Self::Code => "code",
        }
    }
}

fn constant_type(constant: &Constant) -> NativeType {
    match constant {
        Constant::None => NativeType::None,
        Constant::Bool(_) => NativeType::Bool,
        Constant::Ellipsis => NativeType::Ellipsis,
        Constant::Int(_) | Constant::SignedInt(_) | Constant::BigInt { .. } => NativeType::Int,
        Constant::Float(_) => NativeType::Float,
        Constant::Complex { .. } => NativeType::Complex,
        Constant::NonReflexive(number) => constant_type(&number.value()),
        Constant::String(_) | Constant::SurrogateString(_) => NativeType::Str,
        Constant::Bytes(_) => NativeType::Bytes,
        Constant::Tuple(_) => NativeType::Tuple,
        Constant::FrozenSet(_) => NativeType::FrozenSet,
        Constant::Slice { .. } => NativeType::Slice,
        Constant::Code(_) => NativeType::Code,
    }
}

/// The legacy AST pass leaves joined strings and slices at runtime, even inside
/// an otherwise foldable expression. Its recursive unary folding also predates
/// the modern instruction optimizer's handling of `not` and boolean inversion.
fn legacy_constant(expression: &Expr, version: PythonVersion) -> Option<Constant> {
    struct RuntimeExpression(bool);
    impl<'ast> Visitor<'ast> for RuntimeExpression {
        fn visit_interpolated_string_element(
            &mut self,
            element: &'ast ast::InterpolatedStringElement,
        ) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_interpolated_string_element(element));
            }
            ruff_python_ast::visitor::walk_interpolated_string_element(self, element);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if matches!(
                expression,
                Expr::FString(_) | Expr::TString(_) | Expr::Slice(_)
            ) {
                self.0 = true;
            } else if !self.0 {
                walk_expr(self, expression);
            }
        }
    }

    if crate::stack_growth_required() {
        return crate::grow_stack(|| legacy_constant(expression, version));
    }
    match expression {
        Expr::Name(name) if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__" => {
            return Some(Constant::Bool(true));
        }
        Expr::NoneLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::EllipsisLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BytesLiteral(_) => return literal_constant(expression),
        Expr::Tuple(tuple) if tuple.ctx == ExprContext::Load => {}
        Expr::Subscript(subscript) if subscript.ctx == ExprContext::Load => {}
        Expr::UnaryOp(_) | Expr::BinOp(_) => {}
        _ => return None,
    }

    let literal = |expression: &Expr| {
        if let Expr::UnaryOp(unary) = expression {
            if unary.op == UnaryOp::Not {
                return legacy_constant(&unary.operand, version)
                    .map(|constant| Constant::Bool(!constant.is_truthy()));
            }
            if unary.op == UnaryOp::Invert
                && let Some(Constant::Bool(value)) = legacy_constant(&unary.operand, version)
            {
                return Some(Constant::SignedInt(-i64::from(value) - 1));
            }
        }
        literal_constant(expression)
    };
    // Native legacy multiplication accepts bool as an integer repetition count.
    // The callback above handles every constant `not` itself, so enabling Boolean
    // repetition cannot select the shared folder's modern `not(tuple)` rules.
    let constant = fold_constant_with_literals(expression, version, &literal, true)?;
    let mut runtime = RuntimeExpression(false);
    runtime.visit_expr(expression);
    (!runtime.0).then_some(constant)
}

fn preprocessed_constant(expression: &Expr, version: PythonVersion) -> Option<Constant> {
    if version <= PythonVersion::Py313 {
        legacy_constant(expression, version)
    } else if matches!(expression, Expr::Name(name)
        if name.ctx == ExprContext::Load && name.id.as_str() == "__debug__")
    {
        Some(Constant::Bool(true))
    } else {
        literal_constant(expression)
    }
}

fn is_percent_format_string(expression: &ast::ExprBinOp, version: PythonVersion) -> bool {
    if expression.op != Operator::Mod {
        return false;
    }
    if version >= PythonVersion::Py314 {
        return optimized_percent_format(expression, |string| {
            Constant::String(string.value.to_str().to_string())
        })
        .is_some();
    }
    // The old pass folds children first: a constant RHS tuple loses its Tuple
    // AST shape, while a folded LHS string can become eligible for rewriting.
    let Expr::Tuple(arguments) = expression.right.as_ref() else {
        return false;
    };
    if legacy_constant(&expression.right, version).is_some() {
        return false;
    }
    let Some(Constant::String(format)) = legacy_constant(&expression.left, version) else {
        return false;
    };
    optimized_percent_format_arguments(arguments, || Constant::String(format)).is_some()
}

fn infer_type(expression: &Expr, version: PythonVersion) -> Option<NativeType> {
    if let Some(constant) = preprocessed_constant(expression, version) {
        return Some(constant_type(&constant));
    }
    Some(match expression {
        Expr::Tuple(_) => NativeType::Tuple,
        Expr::List(_) | Expr::ListComp(_) => NativeType::List,
        Expr::Dict(_) | Expr::DictComp(_) => NativeType::Dict,
        Expr::Set(_) | Expr::SetComp(_) => NativeType::Set,
        Expr::Generator(_) => NativeType::Generator,
        Expr::Lambda(_) => NativeType::Function,
        Expr::FString(_) => NativeType::Str,
        Expr::TString(_) if version >= PythonVersion::Py314 => NativeType::Template,
        Expr::BinOp(binary) if is_percent_format_string(binary, version) => NativeType::Str,
        _ => return None,
    })
}

fn identity_literal(expression: &Expr, version: PythonVersion) -> Option<NativeType> {
    if let Some(constant) = preprocessed_constant(expression, version) {
        return (!matches!(
            constant,
            Constant::None | Constant::Bool(_) | Constant::Ellipsis
        ))
        .then(|| constant_type(&constant));
    }
    if version >= PythonVersion::Py314
        && let Expr::Tuple(tuple) = expression
        && tuple
            .elts
            .iter()
            .all(|element| preprocessed_constant(element, version).is_some())
    {
        return Some(NativeType::Tuple);
    }
    None
}

/// Classify the first offending identity pair, before lowering any operands.
pub(super) fn compare(expression: &ast::ExprCompare, version: PythonVersion) -> Option<Warning> {
    compare_with_negation(expression, version, false)
}

/// Legacy AST preprocessing folds an odd number of `not` operators into a
/// single identity comparison. Modern targets retain the original operator.
/// The copied native comparison keeps the inner comparison's source range.
pub(super) fn compare_with_negation(
    expression: &ast::ExprCompare,
    version: PythonVersion,
    negated: bool,
) -> Option<Warning> {
    for (operator, operands) in expression.ops.iter().zip(expression.operands.windows(2)) {
        let mut negative = match operator {
            CmpOp::Is => false,
            CmpOp::IsNot => true,
            _ => continue,
        };
        let literal = identity_literal(&operands[0], version)
            .or_else(|| identity_literal(&operands[1], version));
        let Some(literal) = literal else {
            continue;
        };
        if negated && version <= PythonVersion::Py313 && expression.ops.len() == 1 {
            negative = !negative;
        }
        let (operator, replacement) = if negative {
            ("is not", "!=")
        } else {
            ("is", "==")
        };
        return Some(Warning {
            kind: WarningKind::IdentityComparison,
            range: expression.range,
            message: format!(
                "\"{operator}\" with '{}' literal. Did you mean \"{replacement}\"?",
                literal.name()
            ),
        });
    }
    None
}

/// Classify the assertion before any optimization-level early return.
pub(super) fn assertion(statement: &ast::StmtAssert, version: PythonVersion) -> Option<Warning> {
    let nonempty =
        if let Some(Constant::Tuple(values)) = preprocessed_constant(&statement.test, version) {
            !values.is_empty()
        } else {
            matches!(statement.test.as_ref(), Expr::Tuple(tuple) if !tuple.elts.is_empty())
        };
    nonempty.then(|| Warning {
        kind: WarningKind::AssertionAlwaysTrue,
        range: statement.range,
        message: "assertion is always true, perhaps remove parentheses?".to_string(),
    })
}

/// Classify after native keyword validation, but before lowering the callee.
pub(super) fn call(expression: &ast::ExprCall, version: PythonVersion) -> Option<Warning> {
    let callee = infer_type(&expression.func, version)?;
    if matches!(callee, NativeType::Function | NativeType::Slice) {
        return None;
    }
    Some(Warning {
        kind: WarningKind::NonCallable,
        range: expression.func.range(),
        message: format!(
            "'{}' object is not callable; perhaps you missed a comma?",
            callee.name()
        ),
    })
}

/// Both native subscription checks precede lowering the value and index. Their
/// value categories are disjoint, so at most one warning is produced here.
pub(super) fn subscript(
    expression: &ast::ExprSubscript,
    version: PythonVersion,
) -> Option<Warning> {
    if expression.ctx != ExprContext::Load {
        return None;
    }
    let value = infer_type(&expression.value, version)?;
    if matches!(
        value,
        NativeType::None
            | NativeType::Bool
            | NativeType::Ellipsis
            | NativeType::Int
            | NativeType::Float
            | NativeType::Complex
            | NativeType::Set
            | NativeType::FrozenSet
            | NativeType::Generator
            | NativeType::Function
            | NativeType::Template
    ) {
        return Some(Warning {
            kind: WarningKind::NonSubscriptable,
            range: expression.value.range(),
            message: format!(
                "'{}' object is not subscriptable; perhaps you missed a comma?",
                value.name()
            ),
        });
    }
    if !matches!(
        value,
        NativeType::Str | NativeType::Bytes | NativeType::Tuple | NativeType::List
    ) {
        return None;
    }
    let index = infer_type(&expression.slice, version)?;
    if matches!(
        index,
        NativeType::Int | NativeType::Bool | NativeType::Slice
    ) {
        return None;
    }
    Some(Warning {
        kind: WarningKind::InvalidIndex,
        range: expression.value.range(),
        message: format!(
            "{} indices must be integers or slices, not {}; perhaps you missed a comma?",
            value.name(),
            index.name()
        ),
    })
}

/// Classify the current expression without visiting its children. The native
/// schedule decides whether lowering reaches this candidate after preprocessing.
pub(super) fn expression_has_codegen_warning(expression: &Expr, version: PythonVersion) -> bool {
    match expression {
        Expr::Compare(expression) => compare(expression, version).is_some(),
        Expr::Call(expression) => call(expression, version).is_some(),
        Expr::Subscript(expression) => subscript(expression, version).is_some(),
        _ => false,
    }
}

/// Classify the current statement without visiting its expressions or body.
pub(super) fn statement_has_codegen_warning(statement: &Stmt, version: PythonVersion) -> bool {
    matches!(statement, Stmt::Assert(statement) if assertion(statement, version).is_some())
}

/// Exercise candidate collection independently from the source analysis pass.
#[cfg(test)]
pub(super) fn has_codegen_warning_candidates(body: &[Stmt], version: PythonVersion) -> bool {
    struct Candidates {
        version: PythonVersion,
        found: bool,
    }

    impl<'ast> Visitor<'ast> for Candidates {
        fn visit_interpolated_string_element(
            &mut self,
            element: &'ast ast::InterpolatedStringElement,
        ) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_interpolated_string_element(element));
            }
            ruff_python_ast::visitor::walk_interpolated_string_element(self, element);
        }

        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            if self.found {
                return;
            }
            if statement_has_codegen_warning(statement, self.version) {
                self.found = true;
            } else {
                walk_stmt(self, statement);
            }
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if self.found {
                return;
            }
            self.found = expression_has_codegen_warning(expression, self.version);
            if !self.found {
                walk_expr(self, expression);
            }
        }
    }

    let mut candidates = Candidates {
        version,
        found: false,
    };
    candidates.visit_body(body);
    candidates.found
}

/// Find a possible legacy AST-optimization `DeprecationWarning` from inverting a
/// folded Boolean. This is outside the supported `SyntaxWarning` families: an
/// escalated warning can prevent folding, and the optimizer clears its error.
/// This deliberately also reports candidates in unevaluated annotations, so a
/// caller can reject unsupported effects without silently omitting a warning.
pub(super) fn legacy_boolean_inversion(body: &[Stmt], version: PythonVersion) -> Option<TextRange> {
    struct Inversion {
        version: PythonVersion,
        range: Option<TextRange>,
    }

    impl<'ast> Visitor<'ast> for Inversion {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            walk_stmt(self, statement);
        }

        fn visit_pattern(&mut self, pattern: &'ast ast::Pattern) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            ruff_python_ast::visitor::walk_pattern(self, pattern);
        }

        fn visit_interpolated_string_element(
            &mut self,
            element: &'ast ast::InterpolatedStringElement,
        ) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_interpolated_string_element(element));
            }
            ruff_python_ast::visitor::walk_interpolated_string_element(self, element);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_expr(expression));
            }
            if self.range.is_some() {
                return;
            }
            if let Expr::UnaryOp(unary) = expression
                && unary.op == UnaryOp::Invert
                && matches!(
                    legacy_constant(&unary.operand, self.version),
                    Some(Constant::Bool(_))
                )
            {
                self.range = Some(unary.range);
            } else {
                walk_expr(self, expression);
            }
        }
    }

    if version >= PythonVersion::Py314 {
        return None;
    }

    let mut inversion = Inversion {
        version,
        range: None,
    };
    inversion.visit_body(body);
    inversion.range
}

/// Collect only PEP 765 preprocessing warnings, in the native statement walk's
/// order. Expressions contain no statements, so their different visitation
/// order cannot change this warning family. This pass does not collect codegen
/// warnings or duplicate finalizer copies.
pub(super) fn preprocessing(body: &[Stmt], version: PythonVersion) -> Vec<Warning> {
    #[derive(Default)]
    struct FinallyWarnings {
        in_finally: bool,
        warnings: Vec<Warning>,
    }

    impl FinallyWarnings {
        fn body_with_context(&mut self, body: &[Stmt], in_finally: bool) {
            let previous = std::mem::replace(&mut self.in_finally, in_finally);
            self.visit_body(body);
            self.in_finally = previous;
        }

        fn record(&mut self, statement: &Stmt, kind: WarningKind, keyword: &str) {
            if self.in_finally {
                self.warnings.push(Warning {
                    kind,
                    range: statement.range(),
                    message: format!("'{keyword}' in a 'finally' block"),
                });
            }
        }
    }

    impl<'ast> Visitor<'ast> for FinallyWarnings {
        fn visit_pattern(&mut self, _pattern: &'ast ast::Pattern) {}

        fn visit_expr(&mut self, _expression: &'ast Expr) {}

        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            match statement {
                Stmt::FunctionDef(function) => self.body_with_context(&function.body, false),
                // Native preprocessing does not push a new context for classes.
                Stmt::ClassDef(class) => self.visit_body(&class.body),
                Stmt::For(statement) => {
                    self.body_with_context(&statement.body, false);
                    self.visit_body(&statement.orelse);
                }
                Stmt::While(statement) => {
                    self.body_with_context(&statement.body, false);
                    self.visit_body(&statement.orelse);
                }
                Stmt::Try(statement) => {
                    self.visit_body(&statement.body);
                    for handler in &statement.handlers {
                        self.visit_except_handler(handler);
                    }
                    self.visit_body(&statement.orelse);
                    self.body_with_context(&statement.finalbody, true);
                }
                Stmt::Return(_) => self.record(statement, WarningKind::FinallyReturn, "return"),
                Stmt::Break(_) => self.record(statement, WarningKind::FinallyBreak, "break"),
                Stmt::Continue(_) => {
                    self.record(statement, WarningKind::FinallyContinue, "continue");
                }
                _ => walk_stmt(self, statement),
            }
        }
    }

    if version <= PythonVersion::Py313 {
        return Vec::new();
    }

    let mut visitor = FinallyWarnings::default();
    visitor.visit_body(body);
    visitor.warnings
}

#[cfg(test)]
mod tests {
    use super::{
        Warning, WarningKind, assertion, call, compare, compare_with_negation,
        has_codegen_warning_candidates, legacy_boolean_inversion, preprocessing, subscript,
    };
    use crate::PythonVersion;
    use ruff_python_ast::{Expr, Stmt};

    const VERSIONS: [PythonVersion; 4] = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ];

    #[test]
    fn trace_gate_includes_nested_and_unreachable_warning_candidates() {
        for version in VERSIONS {
            for source in [
                "if False:\n    def f():\n        return 1()\n",
                "def f():\n    assert (x,)\n",
                "x = [value for value in xs if value is 1]\n",
                "x = f'{None[0]}'\n",
            ] {
                let parsed = ruff_python_parser::parse_module(source).unwrap();
                assert!(
                    has_codegen_warning_candidates(parsed.suite(), version),
                    "{version}: {source}"
                );
            }
            let parsed = ruff_python_parser::parse_module("x = func(values[0])\n").unwrap();
            assert!(!has_codegen_warning_candidates(parsed.suite(), version));
        }
    }

    #[test]
    fn boolean_inversion_reports_the_unsupported_legacy_precursor() {
        for version in VERSIONS {
            for source in [
                "~True",
                "~__debug__",
                "~(not (1 + 2))",
                "if False:\n    ~False\n",
                "~(not ('a' * True))",
            ] {
                let parsed = ruff_python_parser::parse_module(source).unwrap();
                assert_eq!(
                    legacy_boolean_inversion(parsed.suite(), version).is_some(),
                    version <= PythonVersion::Py313,
                    "{version}: {source}"
                );
            }
            let parsed = ruff_python_parser::parse_module("~1\n~value\n").unwrap();
            assert!(legacy_boolean_inversion(parsed.suite(), version).is_none());
        }
    }

    #[test]
    fn legacy_boolean_repetition_keeps_recursive_not_semantics() {
        for version in VERSIONS {
            let legacy = version <= PythonVersion::Py313;
            for source in [
                "('a' * True)()",
                "(False * b'a')()",
                "((1,) * True)()",
                "(not ((1,) * True))()",
                "(not not not ...)()",
            ] {
                assert_eq!(
                    expression_warning(source, version).is_some(),
                    legacy,
                    "{version}: {source}"
                );
                let parsed = ruff_python_parser::parse_module(source).unwrap();
                assert_eq!(
                    has_codegen_warning_candidates(parsed.suite(), version),
                    legacy,
                    "{version}: {source}"
                );
            }
        }
    }

    fn expression_warning(source: &str, version: PythonVersion) -> Option<Warning> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let Stmt::Expr(statement) = &parsed.suite()[0] else {
            panic!("expected an expression");
        };
        match statement.value.as_ref() {
            Expr::Compare(expression) => compare(expression, version),
            Expr::Call(expression) => call(expression, version),
            Expr::Subscript(expression) => subscript(expression, version),
            _ => panic!("expected a classified expression"),
        }
    }

    #[test]
    fn identity_uses_first_offending_pair_and_prefers_left_type() {
        for version in VERSIONS {
            let warning = expression_warning("1 is 'right' is not 2", version).unwrap();
            assert_eq!(warning.kind, WarningKind::IdentityComparison);
            assert_eq!(
                warning.message,
                "\"is\" with 'int' literal. Did you mean \"==\"?"
            );
            assert_eq!(warning.range, ruff_text_size::TextRange::up_to(21.into()));
            assert!(expression_warning("None is True is ... is False", version).is_none());
            assert_eq!(
                expression_warning("left == 1 is not right", version)
                    .unwrap()
                    .message,
                "\"is not\" with 'int' literal. Did you mean \"!=\"?"
            );
        }
    }

    #[test]
    fn identity_respects_the_target_preprocessing_pass() {
        for version in VERSIONS {
            let legacy = version <= PythonVersion::Py313;
            for source in ["x is -1", "x is 1 + 2", "x is ((1,),)", "x is ~True"] {
                assert_eq!(
                    expression_warning(source, version).is_some(),
                    legacy,
                    "{version}: {source}"
                );
            }
            for source in ["x is ()", "x is (None, __debug__)"] {
                assert!(
                    expression_warning(source, version).is_some(),
                    "{version}: {source}"
                );
            }
            for source in [
                "x is (f'',)",
                "x is f''",
                "x is (1 / 0)",
                "x is (2 ** 1000)",
            ] {
                assert!(
                    expression_warning(source, version).is_none(),
                    "{version}: {source}"
                );
            }
        }
    }

    #[test]
    fn legacy_not_changes_only_a_single_identity_operator() {
        let parsed = ruff_python_parser::parse_module("not (value is 1)").unwrap();
        let Stmt::Expr(statement) = &parsed.suite()[0] else {
            unreachable!()
        };
        let Expr::UnaryOp(unary) = statement.value.as_ref() else {
            unreachable!()
        };
        let Expr::Compare(expression) = unary.operand.as_ref() else {
            unreachable!()
        };
        for version in VERSIONS {
            let warning = compare_with_negation(expression, version, true).unwrap();
            assert_eq!(warning.range, expression.range);
            assert_eq!(
                warning.message.contains("\"is not\""),
                version <= PythonVersion::Py313
            );
        }
    }

    #[test]
    fn call_types_and_ranges_follow_native_ast_shapes() {
        for version in VERSIONS {
            for (source, type_name) in [
                ("None()", "NoneType"),
                ("True()", "bool"),
                ("...()", "ellipsis"),
                ("1j()", "complex"),
                ("[x for x in xs]()", "list"),
                ("(x for x in xs)()", "generator"),
                ("f''()", "str"),
                ("__debug__()", "bool"),
            ] {
                let warning = expression_warning(source, version).unwrap();
                assert_eq!(
                    warning.message,
                    format!("'{type_name}' object is not callable; perhaps you missed a comma?")
                );
                assert!(warning.range.end().to_usize() < source.len());
            }
            assert!(expression_warning("(lambda: 1)()", version).is_none());
            assert_eq!(
                expression_warning("(-1)()", version).is_some(),
                version <= PythonVersion::Py313
            );
        }
    }

    #[test]
    fn percent_format_rewriting_precedes_classification() {
        for version in VERSIONS {
            let legacy = version <= PythonVersion::Py313;
            assert_eq!(
                expression_warning("('%s' % (1,))()", version).is_some(),
                !legacy
            );
            assert!(expression_warning("('%s' % (value,))()", version).is_some());
            assert_eq!(
                expression_warning("(('%' + 's') % (value,))()", version).is_some(),
                legacy
            );
            assert!(expression_warning("('%d' % (value,))()", version).is_none());
        }
    }

    #[test]
    fn subscript_categories_and_integer_exceptions() {
        for version in VERSIONS {
            for source in ["None[0]", "True[0]", "{1}[0]", "(lambda: 1)[0]"] {
                assert_eq!(
                    expression_warning(source, version).unwrap().kind,
                    WarningKind::NonSubscriptable
                );
            }
            let warning = expression_warning("b'abc'[None]", version).unwrap();
            assert_eq!(warning.kind, WarningKind::InvalidIndex);
            assert_eq!(
                warning.message,
                "bytes indices must be integers or slices, not NoneType; perhaps you missed a comma?"
            );
            assert_eq!(warning.range, ruff_text_size::TextRange::up_to(6.into()));
            for source in ["[1][True]", "[1][0:1]", "{}[None]", "value[None]"] {
                assert!(
                    expression_warning(source, version).is_none(),
                    "{version}: {source}"
                );
            }
            assert_eq!(
                expression_warning("[1][-1.5]", version).is_some(),
                version <= PythonVersion::Py313
            );
        }
    }

    #[test]
    fn subscript_targets_do_not_warn() {
        for source in ["None[0] = 1", "del None[0]", "None[0] += 1"] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            let target = match &parsed.suite()[0] {
                Stmt::Assign(statement) => &statement.targets[0],
                Stmt::Delete(statement) => &statement.targets[0],
                Stmt::AugAssign(statement) => statement.target.as_ref(),
                _ => unreachable!(),
            };
            let Expr::Subscript(expression) = target else {
                unreachable!()
            };
            for version in VERSIONS {
                assert!(subscript(expression, version).is_none());
            }
        }
    }

    #[test]
    fn assert_warns_for_nonempty_and_legacy_folded_tuples() {
        for (source, expected) in [
            ("assert ()", [false; 4]),
            ("assert (x,)", [true; 4]),
            ("assert (1,) * 2", [true, true, false, false]),
        ] {
            let parsed = ruff_python_parser::parse_module(source).unwrap();
            let Stmt::Assert(statement) = &parsed.suite()[0] else {
                unreachable!()
            };
            for (version, expected) in VERSIONS.into_iter().zip(expected) {
                assert_eq!(
                    assertion(statement, version).is_some(),
                    expected,
                    "{version}: {source}"
                );
            }
        }
    }

    #[test]
    fn templates_use_the_native_type_name() {
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            assert_eq!(
                expression_warning("t''()", version).unwrap().message,
                "'string.templatelib.Template' object is not callable; perhaps you missed a comma?"
            );
            assert_eq!(
                expression_warning("t''[0]", version).unwrap().kind,
                WarningKind::NonSubscriptable
            );
            assert_eq!(
                expression_warning("[1][t'']", version).unwrap().kind,
                WarningKind::InvalidIndex
            );
        }
    }

    #[test]
    fn preprocessing_follows_top_context_and_statement_order() {
        let source = "def f():\n    try:\n        pass\n    finally:\n        for x in xs:\n            return 1\n        else:\n            return 2\n        def inner():\n            return 3\n        try:\n            pass\n        finally:\n            break\n            continue\n        return 4\n";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        for version in VERSIONS {
            let warnings = preprocessing(parsed.suite(), version);
            if version <= PythonVersion::Py313 {
                assert!(warnings.is_empty());
                continue;
            }
            assert_eq!(
                warnings
                    .iter()
                    .map(|warning| warning.kind)
                    .collect::<Vec<_>>(),
                [
                    WarningKind::FinallyReturn,
                    WarningKind::FinallyBreak,
                    WarningKind::FinallyContinue,
                    WarningKind::FinallyReturn
                ]
            );
            assert_eq!(
                warnings
                    .iter()
                    .map(|warning| &source[warning.range])
                    .collect::<Vec<_>>(),
                ["return 2", "break", "continue", "return 4"]
            );
        }
    }

    #[test]
    fn preprocessing_visits_unreachable_statements_and_does_not_reset_at_classes() {
        let source = "try:\n    pass\nfinally:\n    if False:\n        return 1\n    class C:\n        return 2\n";
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        let warnings = preprocessing(parsed.suite(), PythonVersion::Py314);
        assert_eq!(
            warnings
                .iter()
                .map(|warning| &source[warning.range])
                .collect::<Vec<_>>(),
            ["return 1", "return 2"]
        );
        assert!(
            warnings
                .iter()
                .all(|warning| warning.message == "'return' in a 'finally' block")
        );
    }
}
