//! Native warning invocations ordered against code-object assembly.
//!
//! CPython generates every statement before CFG pruning, so source locations
//! alone cannot order warnings: class bodies precede bases, and 3.12/3.13
//! generate a while condition twice. This planner follows the pinned native
//! codegen visits and projects native unwind frames onto warning/assembly events.
//! Unmodeled synthetic scopes are rejected rather than assigned an approximate order.

use std::fmt;

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{Expr, Parameters, Stmt, UnaryOp};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::code_object::CodeObject;
use crate::{CompilationWarningPhase, PythonVersion};

use super::warnings::{self, Warning};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AssemblyBoundary {
    pub(super) name: String,
    pub(super) qualname: String,
    pub(super) firstlineno: u32,
}

impl AssemblyBoundary {
    /// Match the next native assembly without relying on lowering's scope IDs.
    pub(super) fn matches(&self, code: &CodeObject) -> bool {
        self.name == code.name
            && self.qualname == code.qualified_name
            && self.firstlineno == code.first_line_number
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ScheduleEvent {
    Warning {
        phase: CompilationWarningPhase,
        warning: Warning,
    },
    Assembly(AssemblyBoundary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UnsupportedWarningSchedule {
    pub(super) range: TextRange,
    pub(super) reason: &'static str,
}

impl fmt::Display for UnsupportedWarningSchedule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {:?}", self.reason, self.range)
    }
}

#[derive(Clone, Debug)]
pub(super) struct WarningSchedule {
    pub(super) events: Vec<ScheduleEvent>,
}

impl WarningSchedule {
    /// Plan successful native compilation of this exact decoded source and AST.
    ///
    /// Parser warnings have already occurred and are supplied by the parser's
    /// separate source-bound plan. Preprocessing precedes every codegen event.
    /// Consumers must match every assembly and reject any unconsumed event.
    pub(super) fn plan(
        body: &[Stmt],
        source: &str,
        version: PythonVersion,
    ) -> Result<Self, UnsupportedWarningSchedule> {
        if let Some(range) = warnings::legacy_boolean_inversion(body, version) {
            return Err(UnsupportedWarningSchedule {
                range,
                reason: "legacy boolean-inversion preprocessing warning",
            });
        }
        let mut planner = Planner {
            source,
            version,
            events: warnings::preprocessing(body, version)
                .into_iter()
                .map(|warning| ScheduleEvent::Warning {
                    phase: CompilationWarningPhase::Preprocessing,
                    warning,
                })
                .collect(),
            scope: Scope::Module,
            qualname: String::new(),
            unsupported: None,
            frames: Vec::new(),
            disabled_warnings: 0,
        };
        planner.visit_body(body);
        if let Some(unsupported) = planner.unsupported {
            return Err(unsupported);
        }
        planner
            .events
            .push(ScheduleEvent::Assembly(AssemblyBoundary {
                name: "<module>".to_string(),
                qualname: "<module>".to_string(),
                firstlineno: 1,
            }));
        Ok(Self {
            events: planner.events,
        })
    }
}

#[derive(Clone, Copy)]
enum Scope {
    Module,
    Function,
    Class,
}

#[derive(Clone, Copy)]
enum Frame<'ast> {
    Loop,
    FinallyTry(&'ast [Stmt]),
    FinallyEnd,
}

struct Planner<'source, 'ast> {
    source: &'source str,
    version: PythonVersion,
    events: Vec<ScheduleEvent>,
    scope: Scope,
    qualname: String,
    unsupported: Option<UnsupportedWarningSchedule>,
    frames: Vec<Frame<'ast>>,
    disabled_warnings: usize,
}

impl<'ast> Planner<'_, 'ast> {
    /// Stop at the first construct whose native visitation is not modeled.
    fn unsupported(&mut self, range: TextRange, reason: &'static str) {
        self.unsupported
            .get_or_insert(UnsupportedWarningSchedule { range, reason });
    }

    fn warning(&mut self, warning: Option<Warning>) {
        if self.disabled_warnings == 0
            && let Some(warning) = warning
        {
            self.events.push(ScheduleEvent::Warning {
                phase: CompilationWarningPhase::CodeGeneration,
                warning,
            });
        }
    }

    /// Count physical source lines, including lone CR and CRLF newlines.
    fn line(&self, offset: TextSize) -> u32 {
        let bytes = &self.source.as_bytes()[..usize::from(offset)];
        let mut line = 1;
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\r' || (*byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r')) {
                line += 1;
            }
        }
        line
    }

    /// Assemble a child after visiting its complete body, then restore its parent.
    fn child_scope(
        &mut self,
        name: &str,
        first_line: TextSize,
        scope: Scope,
        visit: impl FnOnce(&mut Self),
    ) {
        let qualname = match self.scope {
            Scope::Module => name.to_string(),
            Scope::Function => format!("{}.<locals>.{name}", self.qualname),
            Scope::Class => format!("{}.{name}", self.qualname),
        };
        let parent_scope = std::mem::replace(&mut self.scope, scope);
        let parent_name = std::mem::replace(&mut self.qualname, qualname);
        // Warning disabling belongs to the compiler, but unwind frames belong
        // to its current unit. A nested function in an exceptional finalizer
        // inherits disabling without inheriting the enclosing unwind frames.
        let parent_frames = std::mem::take(&mut self.frames);
        visit(self);
        self.frames = parent_frames;
        self.events.push(ScheduleEvent::Assembly(AssemblyBoundary {
            name: name.to_string(),
            qualname: std::mem::replace(&mut self.qualname, parent_name),
            firstlineno: self.line(first_line),
        }));
        self.scope = parent_scope;
    }

    /// Defaults are visited in positional-then-keyword order in the parent unit.
    fn defaults(&mut self, parameters: &'ast Parameters) {
        for parameter in parameters.iter_non_variadic_params() {
            if let Some(default) = &parameter.default {
                self.visit_expr(default);
            }
        }
    }

    /// Push a projected native frame, preserving compiler-global warning state.
    fn push_frame(&mut self, frame: Frame<'ast>) {
        if matches!(frame, Frame::FinallyEnd) && self.version >= PythonVersion::Py313 {
            self.disabled_warnings += 1;
        }
        self.frames.push(frame);
    }

    fn pop_frame(&mut self) -> Frame<'ast> {
        let frame = self.frames.pop().expect("balanced native compile frames");
        if matches!(frame, Frame::FinallyEnd) && self.version >= PythonVersion::Py313 {
            self.disabled_warnings -= 1;
        }
        frame
    }

    /// Native unwind pops before visiting each finalizer and restores afterward.
    ///
    /// Other native frames only emit instructions: they neither visit source AST
    /// nor change warning disabling or the loop boundary, so are absent here.
    fn unwind(&mut self, stop_at_loop: bool) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.unwind(stop_at_loop));
        }
        let Some(frame) = self.frames.last().copied() else {
            return;
        };
        if stop_at_loop && matches!(frame, Frame::Loop) {
            return;
        }
        // 3.12/3.13 mutate u_nfblocks directly while unwinding. In 3.13
        // this deliberately leaves c_disable_warning unchanged. Modern
        // codegen calls the normal pop/push helpers, updating it immediately.
        if self.version <= PythonVersion::Py313 {
            self.frames.pop();
        } else {
            self.pop_frame();
        }
        if let Frame::FinallyTry(body) = frame {
            self.visit_body(body);
        }
        self.unwind(stop_at_loop);
        if self.version <= PythonVersion::Py313 {
            self.frames.push(frame);
        } else {
            self.push_frame(frame);
        }
    }

    /// The native nonexceptional else path is generated before handler bodies.
    fn try_body(&mut self, statement: &'ast ruff_python_ast::StmtTry) {
        self.visit_body(&statement.body);
        self.visit_body(&statement.orelse);
        for handler in &statement.handlers {
            self.visit_except_handler(handler);
        }
    }
}

impl<'ast> Visitor<'ast> for Planner<'_, 'ast> {
    fn visit_interpolated_string_element(
        &mut self,
        element: &'ast ruff_python_ast::InterpolatedStringElement,
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
        if self.unsupported.is_some() {
            return;
        }
        match statement {
            Stmt::FunctionDef(function) => {
                if function.type_params.is_some()
                    || function.returns.is_some()
                    || function
                        .parameters
                        .iter()
                        .any(|parameter| parameter.as_parameter().annotation.is_some())
                {
                    self.unsupported(
                        function.range,
                        "function annotation or type-parameter scope",
                    );
                    return;
                }
                for decorator in &function.decorator_list {
                    self.visit_expr(&decorator.expression);
                }
                self.defaults(&function.parameters);
                let first_line = function
                    .decorator_list
                    .first()
                    .map_or(function.start(), |decorator| decorator.expression.start());
                self.child_scope(
                    function.name.as_str(),
                    first_line,
                    Scope::Function,
                    |this| {
                        this.visit_body(&function.body);
                    },
                );
            }
            Stmt::ClassDef(class) => {
                if class.type_params.is_some() {
                    self.unsupported(class.range, "class type-parameter scope");
                    return;
                }
                for decorator in &class.decorator_list {
                    self.visit_expr(&decorator.expression);
                }
                let first_line = class
                    .decorator_list
                    .first()
                    .map_or(class.start(), |decorator| decorator.expression.start());
                self.child_scope(class.name.as_str(), first_line, Scope::Class, |this| {
                    this.visit_body(&class.body);
                });
                // Native codegen_class assembles the body before visiting bases.
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
            }
            Stmt::Assert(assertion) => {
                self.warning(warnings::assertion(assertion, self.version));
                walk_stmt(self, statement);
            }
            Stmt::AugAssign(assignment) => {
                // Augmented stores lower the target children before the RHS,
                // without invoking the ordinary subscription warning checks.
                match assignment.target.as_ref() {
                    Expr::Attribute(attribute) => self.visit_expr(&attribute.value),
                    Expr::Subscript(subscript) => {
                        self.visit_expr(&subscript.value);
                        self.visit_expr(&subscript.slice);
                    }
                    Expr::Name(_) => {}
                    _ => {
                        self.unsupported(assignment.range, "invalid augmented-assignment target");
                        return;
                    }
                }
                self.visit_expr(&assignment.value);
            }
            Stmt::While(statement) => {
                self.push_frame(Frame::Loop);
                self.visit_expr(&statement.test);
                self.visit_body(&statement.body);
                if self.version <= PythonVersion::Py313 {
                    self.visit_expr(&statement.test);
                }
                self.pop_frame();
                self.visit_body(&statement.orelse);
            }
            Stmt::For(statement) => {
                self.visit_expr(&statement.iter);
                self.push_frame(Frame::Loop);
                self.visit_expr(&statement.target);
                self.visit_body(&statement.body);
                self.pop_frame();
                self.visit_body(&statement.orelse);
            }
            Stmt::Return(statement) => {
                if let Some(value) = &statement.value {
                    self.visit_expr(value);
                }
                self.unwind(false);
            }
            Stmt::Break(_) | Stmt::Continue(_) => self.unwind(true),
            Stmt::AnnAssign(_) | Stmt::TypeAlias(_) => {
                self.unsupported(statement.range(), "annotation or type-alias scope");
            }
            Stmt::Try(statement) => {
                if statement.is_star {
                    self.unsupported(statement.range, "exception-group handler scheduling");
                } else if statement.finalbody.is_empty() {
                    self.try_body(statement);
                } else {
                    self.push_frame(Frame::FinallyTry(&statement.finalbody));
                    self.try_body(statement);
                    self.pop_frame();
                    self.visit_body(&statement.finalbody);
                    self.push_frame(Frame::FinallyEnd);
                    self.visit_body(&statement.finalbody);
                    self.pop_frame();
                }
            }
            Stmt::Match(_) => {
                self.unsupported(statement.range(), "pattern-matching codegen scheduling");
            }
            Stmt::Global(_) | Stmt::Nonlocal(_) => {
                self.unsupported(
                    statement.range(),
                    "explicit scope declaration and qualified names",
                );
            }
            Stmt::IpyEscapeCommand(_) => {
                self.unsupported(statement.range(), "non-Python statement");
            }
            Stmt::Delete(_)
            | Stmt::Assign(_)
            | Stmt::If(_)
            | Stmt::Raise(_)
            | Stmt::With(_)
            | Stmt::Import(_)
            | Stmt::ImportFrom(_)
            | Stmt::Expr(_)
            | Stmt::Pass(_) => walk_stmt(self, statement),
        }
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_expr(expression));
        }
        if self.unsupported.is_some() {
            return;
        }
        match expression {
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.defaults(parameters);
                }
                self.child_scope("<lambda>", lambda.start(), Scope::Function, |this| {
                    this.visit_expr(&lambda.body);
                });
                return;
            }
            Expr::UnaryOp(unary)
                if self.version <= PythonVersion::Py313 && unary.op == UnaryOp::Not =>
            {
                // The old AST pass toggles a single identity operator for each
                // enclosing `not`, retaining the comparison's original range.
                let mut operand = expression;
                let mut negated = false;
                while let Expr::UnaryOp(unary) = operand {
                    if unary.op != UnaryOp::Not {
                        break;
                    }
                    negated = !negated;
                    operand = &unary.operand;
                }
                if let Expr::Compare(compare) = operand
                    && compare.ops.len() == 1
                {
                    self.warning(warnings::compare_with_negation(
                        compare,
                        self.version,
                        negated,
                    ));
                    walk_expr(self, operand);
                    return;
                }
            }
            Expr::Compare(compare) => self.warning(warnings::compare(compare, self.version)),
            Expr::Call(call) => self.warning(warnings::call(call, self.version)),
            Expr::Subscript(subscript) => {
                self.warning(warnings::subscript(subscript, self.version));
            }
            Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                self.unsupported(
                    expression.range(),
                    "comprehension codegen and synthetic scopes",
                );
                return;
            }
            Expr::IpyEscapeCommand(_) => {
                self.unsupported(expression.range(), "non-Python expression");
                return;
            }
            _ => {}
        }
        walk_expr(self, expression);
    }
}

#[cfg(test)]
mod tests {
    use super::{ScheduleEvent, WarningSchedule};
    use crate::PythonVersion;

    const VERSIONS: [PythonVersion; 4] = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ];

    fn events(source: &str, version: PythonVersion) -> Vec<ScheduleEvent> {
        let parsed = ruff_python_parser::parse_module(source).unwrap();
        WarningSchedule::plan(parsed.suite(), source, version)
            .unwrap()
            .events
    }

    fn summary(source: &str, version: PythonVersion) -> Vec<String> {
        events(source, version)
            .into_iter()
            .map(|event| match event {
                ScheduleEvent::Warning { warning, .. } => {
                    format!("warning:{}", warning.kind.as_str())
                }
                ScheduleEvent::Assembly(boundary) => {
                    format!("assembly:{}:{}", boundary.qualname, boundary.firstlineno)
                }
            })
            .collect()
    }

    #[test]
    fn class_body_is_assembled_before_base_expression() {
        let source = "class Outer(1()):\n    def method():\n        return 2 is x\n";
        for version in VERSIONS {
            assert_eq!(
                summary(source, version),
                [
                    "warning:identity_comparison",
                    "assembly:Outer.method:2",
                    "assembly:Outer:1",
                    "warning:non_callable",
                    "assembly:<module>:1",
                ]
            );
        }
    }

    #[test]
    fn decorators_and_defaults_precede_body_but_unreachable_warnings_remain() {
        let source =
            "@1()\ndef outer(value=2()):\n    return\n    def inner():\n        3()\n    4()\n";
        for version in VERSIONS {
            assert_eq!(
                summary(source, version),
                [
                    "warning:non_callable",
                    "warning:non_callable",
                    "warning:non_callable",
                    "assembly:outer.<locals>.inner:4",
                    "warning:non_callable",
                    "assembly:outer:1",
                    "assembly:<module>:1",
                ]
            );
        }
    }

    #[test]
    fn while_condition_has_target_specific_repeated_invocations() {
        let source = "while 1 is x:\n    2()\nelse:\n    3[0]\n";
        for version in VERSIONS {
            let mut expected = vec!["warning:identity_comparison", "warning:non_callable"];
            if version <= PythonVersion::Py313 {
                expected.push("warning:identity_comparison");
            }
            expected.extend(["warning:non_subscriptable", "assembly:<module>:1"]);
            assert_eq!(summary(source, version), expected);
        }
    }

    #[test]
    fn outer_warning_precedes_nested_lambda_assembly() {
        for version in VERSIONS {
            assert_eq!(
                summary("(lambda: 1())[None]", version),
                [
                    "warning:non_subscriptable",
                    "warning:non_callable",
                    "assembly:<lambda>:1",
                    "assembly:<module>:1",
                ]
            );
        }
    }

    #[test]
    fn starred_arguments_precede_keywords_and_augmented_stores_bypass_root_check() {
        for version in VERSIONS {
            assert_eq!(
                summary("f(keyword=1(), *((2 is x),))\n1[2()] += 3()\n", version),
                [
                    "warning:identity_comparison",
                    "warning:non_callable",
                    "warning:non_callable",
                    "warning:non_callable",
                    "assembly:<module>:1",
                ]
            );
        }
    }

    #[test]
    fn legacy_negation_preserves_occurrence_ranges_and_operator_polarity() {
        for version in VERSIONS {
            let source = "not (1 is x)\nnot not (2 is y)\n";
            let warnings: Vec<_> = events(source, version)
                .into_iter()
                .filter_map(|event| match event {
                    ScheduleEvent::Warning { warning, .. } => Some(warning),
                    ScheduleEvent::Assembly(_) => None,
                })
                .collect();
            assert_eq!(warnings.len(), 2);
            assert_eq!(
                warnings[0].message.starts_with("\"is not\""),
                version <= PythonVersion::Py313,
            );
            assert!(warnings[1].message.starts_with("\"is\""));
            assert_eq!(&source[warnings[0].range], "1 is x");
            assert_eq!(&source[warnings[1].range], "2 is y");
        }
    }

    #[test]
    fn finalizers_repeat_on_exits_and_native_exceptional_copies() {
        let source = "def f():\n    try:\n        return\n    finally:\n        1()\n";
        for version in VERSIONS {
            let mut expected = vec!["warning:non_callable", "warning:non_callable"];
            if version == PythonVersion::Py312 {
                expected.push("warning:non_callable");
            }
            expected.extend(["assembly:f:1", "assembly:<module>:1"]);
            assert_eq!(summary(source, version), expected);
        }
    }

    #[test]
    fn exceptional_copy_disables_nested_scope_warnings_but_still_assembles() {
        let source = "def f():\n    try:\n        return\n    finally:\n        def g():\n            1()\n        2()\n";
        for version in VERSIONS {
            let mut expected = vec![
                "warning:non_callable",
                "assembly:f.<locals>.g:5",
                "warning:non_callable",
                "warning:non_callable",
                "assembly:f.<locals>.g:5",
                "warning:non_callable",
            ];
            if version == PythonVersion::Py312 {
                expected.push("warning:non_callable");
            }
            expected.push("assembly:f.<locals>.g:5");
            if version == PythonVersion::Py312 {
                expected.push("warning:non_callable");
            }
            expected.extend(["assembly:f:1", "assembly:<module>:1"]);
            assert_eq!(summary(source, version), expected);
        }
    }

    #[test]
    fn popping_exceptional_frame_has_target_specific_warning_state() {
        let source = "def f():\n    try:\n        try:\n            pass\n        finally:\n            return\n    finally:\n        1()\n";
        for version in VERSIONS {
            let mut expected = Vec::new();
            if version >= PythonVersion::Py314 {
                expected.push("warning:finally_return");
            }
            let occurrences = match version {
                PythonVersion::Py312 => 4,
                PythonVersion::Py313 => 2,
                PythonVersion::Py314 | PythonVersion::Py315 => 3,
            };
            expected.extend(std::iter::repeat_n("warning:non_callable", occurrences));
            expected.extend(["assembly:f:1", "assembly:<module>:1"]);
            assert_eq!(summary(source, version), expected);
        }
    }

    #[test]
    fn loop_exits_stop_unwinding_at_the_innermost_loop() {
        let source = "while flag:\n    try:\n        if flag:\n            break\n        continue\n    finally:\n        1()\n";
        for version in VERSIONS {
            let mut expected = vec!["warning:non_callable"; 3];
            if version == PythonVersion::Py312 {
                expected.push("warning:non_callable");
            }
            expected.push("assembly:<module>:1");
            assert_eq!(summary(source, version), expected);
        }
    }

    #[test]
    fn prepared_compilation_consumes_native_scope_and_finalizer_schedules() {
        use crate::{
            CompilationWarningEffect, CompileOptions, InitialInternState, PreparedCompilation,
        };

        let cases = [
            (
                "class Outer(1()):\n    def method():\n        return 2 is x\n",
                [2, 2, 2, 2],
            ),
            (
                "@1()\ndef outer(value=2()):\n    return\n    def inner():\n        3()\n    4()\n",
                [4, 4, 4, 4],
            ),
            ("while (lambda: 1())():\n    pass\n", [2, 2, 1, 1]),
            (
                "def f():\n    try:\n        return\n    finally:\n        1()\n",
                [3, 2, 2, 2],
            ),
            (
                "def f():\n    try:\n        return\n    finally:\n        def g():\n            1()\n        2()\n",
                [6, 4, 4, 4],
            ),
            (
                "def f():\n    try:\n        try:\n            pass\n        finally:\n            return\n    finally:\n        1()\n",
                [4, 2, 4, 4],
            ),
            (
                "while flag:\n    try:\n        if flag:\n            break\n        continue\n    finally:\n        1()\n",
                [4, 3, 3, 3],
            ),
            (
                "try:\n    1()\nexcept (2 is x):\n    3[0]\nelse:\n    assert (4,)\n",
                [4, 4, 4, 4],
            ),
        ];
        for (source, counts) in cases {
            for (version, count) in VERSIONS.into_iter().zip(counts) {
                let options = CompileOptions {
                    python_version: version,
                    initial_intern_state: InitialInternState::from_complete(
                        std::iter::empty::<&str>(),
                    ),
                    ..CompileOptions::default()
                };
                let plan = PreparedCompilation::text(source, "warning_schedule.py", &options)
                    .unwrap_or_else(|error| panic!("{version:?}: {error:?}\n{source}"));
                assert_eq!(plan.warnings().len(), count, "{version:?}: {source}");
                let mut effects = plan.effects();
                for ordinal in 0..count {
                    effects
                        .record(ordinal, CompilationWarningEffect::NoRetainedAdditions)
                        .unwrap();
                }
                let finished = plan
                    .finish(&effects)
                    .unwrap_or_else(|error| panic!("{version:?}: {error:?}\n{source}"));
                assert!(!finished.marshal().is_empty());
            }
        }
    }

    #[test]
    fn try_else_is_generated_before_handlers() {
        let source = "try:\n    1()\nexcept (2 is x):\n    3[0]\nelse:\n    assert (4,)\n";
        for version in VERSIONS {
            assert_eq!(
                summary(source, version),
                [
                    "warning:non_callable",
                    "warning:assertion_always_true",
                    "warning:identity_comparison",
                    "warning:non_subscriptable",
                    "assembly:<module>:1",
                ]
            );
        }
    }
}
