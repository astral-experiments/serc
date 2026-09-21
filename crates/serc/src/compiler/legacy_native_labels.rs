//! Exact source labels for a deliberately bounded set of native 3.12 codegen routes.
//!
//! Native labels count allocation events, including unused labels. Their aliases are
//! resolved at the original instruction boundaries, before Serc's lowering drops
//! pseudo-ops or copies exits. Unsupported source or an unbound lowering role cancels
//! the complete code object; it never supplies a partial seed to the assembler.

use ruff_python_ast::{BoolOp, Expr, FStringPart, InterpolatedStringElement, Stmt, UnaryOp};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap;

use crate::assembler::Label;

use super::Compiler;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum NativeRole {
    FunctionStart,
    StopIterationHandler,
    IfEnd(TextRange),
    IfElse(TextRange),
    AssertEnd(TextRange),
    ValueBoolEnd(TextRange),
    ConditionBoolEnd(TextRange),
    ValueIfEnd(TextRange),
    ValueIfElse(TextRange),
    ConditionIfEnd(TextRange),
    ConditionIfElse(TextRange),
    ForStart(TextRange),
    ForBody(TextRange),
    ForCleanup(TextRange),
    ForEnd(TextRange),
    AwaitSend(TextRange),
    AwaitFail(TextRange),
    AwaitExit(TextRange),
    WithBlock(TextRange),
    WithFinal(TextRange),
    WithExit(TextRange),
    WithCleanup(TextRange),
    WithSuppress(TextRange),
    WithHelperExit(TextRange),
    WithAwaitSend(TextRange, WithAwait),
    WithAwaitFail(TextRange, WithAwait),
    WithAwaitExit(TextRange, WithAwait),
}

/// Distinguishes the three implicit awaits of one original async manager.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum WithAwait {
    Enter,
    NormalExit,
    ExceptionalExit,
}

impl WithAwait {
    pub(super) fn roles(self, range: TextRange) -> [NativeRole; 3] {
        [
            NativeRole::WithAwaitSend(range, self),
            NativeRole::WithAwaitFail(range, self),
            NativeRole::WithAwaitExit(range, self),
        ]
    }
}

#[derive(Debug)]
pub(super) struct NativeLabelPlan {
    remaining: FxHashMap<NativeRole, u32>,
    valid: bool,
}

#[derive(Default)]
struct Builder {
    // Only equality matters: advance across original instructions, including
    // instructions that later disappear. This is not a native label ordinal.
    boundary: usize,
    labels: Vec<(NativeRole, Option<usize>)>,
    manager_depth: usize,
}

impl NativeLabelPlan {
    pub(super) fn function(body: &[Stmt], coroutine: bool) -> Option<Box<Self>> {
        let mut builder = Builder::default();
        let start = builder.allocate(NativeRole::FunctionStart);
        builder.mark(start);
        let body = if matches!(body.first(), Some(Stmt::Expr(expression))
            if expression.value.is_string_literal_expr())
        {
            &body[1..]
        } else {
            body
        };
        builder.suite(body)?;
        if coroutine {
            // wrap_in_stopiteration_handler reserves this after the entire body,
            // then inserts SETUP_CLEANUP at offset zero. That insertion shifts all
            // prior marks equally, so it cannot change their alias equivalence.
            let handler = builder.allocate(NativeRole::StopIterationHandler);
            builder.instruction(); // unconditional LOAD_CONST / RETURN_VALUE footer
            builder.mark(handler);
            builder.instruction();
        }
        let mut aliases = FxHashMap::default();
        for (index, (_, boundary)) in builder.labels.iter().enumerate() {
            let ordinal = u32::try_from(index.checked_add(1)?).ok()?;
            aliases.insert((*boundary)?, ordinal);
        }
        let mut remaining = FxHashMap::default();
        for (role, boundary) in builder.labels {
            if remaining.insert(role, aliases[&boundary?]).is_some() {
                return None;
            }
        }
        Some(Box::new(Self {
            remaining,
            valid: true,
        }))
    }
}

impl Builder {
    fn allocate(&mut self, role: NativeRole) -> usize {
        let label = self.labels.len();
        self.labels.push((role, None));
        label
    }

    fn mark(&mut self, label: usize) {
        self.labels[label].1 = Some(self.boundary);
    }

    fn instruction(&mut self) {
        self.boundary += 1;
    }

    fn suite(&mut self, statements: &[Stmt]) -> Option<()> {
        for statement in statements {
            match statement {
                Stmt::Assign(assignment) => {
                    self.expression(&assignment.value)?;
                    for target in &assignment.targets {
                        self.store_target(target)?;
                    }
                }
                Stmt::AnnAssign(assignment)
                    if matches!(assignment.target.as_ref(), Expr::Name(_)) =>
                {
                    // Function-local annotations are checked but never evaluated.
                    // An annotation without a value emits no instruction at all.
                    if let Some(value) = &assignment.value {
                        self.expression(value)?;
                        self.instruction();
                    }
                }
                Stmt::Import(_) | Stmt::ImportFrom(_) => self.instruction(),
                Stmt::Delete(deletion) => {
                    for target in &deletion.targets {
                        if matches!(target, Expr::Tuple(_) | Expr::List(_) | Expr::Starred(_)) {
                            return None;
                        }
                        self.store_target(target)?;
                    }
                }
                Stmt::AugAssign(assignment)
                    if matches!(assignment.target.as_ref(), Expr::Name(_)) =>
                {
                    self.instruction(); // original load of the augmented target
                    self.expression(&assignment.value)?;
                    self.instruction();
                }
                Stmt::Expr(expression) => {
                    self.expression(&expression.value)?;
                    self.instruction(); // POP_TOP, or the constant statement's NOP
                }
                Stmt::Return(statement) => {
                    if self.manager_depth != 0 {
                        return None;
                    }
                    if let Some(value) = &statement.value {
                        self.expression(value)?;
                    }
                    self.instruction();
                }
                Stmt::Raise(statement) => {
                    if let Some(exception) = &statement.exc {
                        self.expression(exception)?;
                    }
                    if let Some(cause) = &statement.cause {
                        self.expression(cause)?;
                    }
                    self.instruction();
                }
                Stmt::For(statement)
                    if !statement.is_async
                        && matches!(statement.target.as_ref(), Expr::Name(_)) =>
                {
                    // compiler_for reserves all four labels before visiting the
                    // iterable. Its body mark follows the original target NOP.
                    let start = self.allocate(NativeRole::ForStart(statement.range));
                    let body = self.allocate(NativeRole::ForBody(statement.range));
                    let cleanup = self.allocate(NativeRole::ForCleanup(statement.range));
                    let end = self.allocate(NativeRole::ForEnd(statement.range));
                    self.expression(&statement.iter)?;
                    self.instruction(); // GET_ITER
                    self.mark(start);
                    self.instruction(); // FOR_ITER
                    self.instruction(); // target-line NOP
                    self.mark(body);
                    self.store_target(&statement.target)?;
                    self.suite(&statement.body)?;
                    self.instruction(); // original backedge, including dead source
                    self.mark(cleanup);
                    self.instruction(); // END_FOR
                    self.suite(&statement.orelse)?;
                    self.mark(end);
                }
                Stmt::Assert(statement) => {
                    let end = self.allocate(NativeRole::AssertEnd(statement.range));
                    self.condition(&statement.test, true)?;
                    self.instruction(); // LOAD_ASSERTION_ERROR
                    if let Some(message) = &statement.msg {
                        self.expression(message)?;
                        self.instruction(); // CALL
                    }
                    self.instruction(); // RAISE_VARARGS
                    self.mark(end);
                }
                Stmt::With(statement) => self.manager(statement)?,
                Stmt::FunctionDef(definition) => {
                    if definition.type_params.is_some() {
                        return None;
                    }
                    // The child's codegen counter is independent. This subset
                    // admits only label-free evaluation in the enclosing scope,
                    // so defaults/decorators/annotation order cannot change IDs.
                    for decorator in &definition.decorator_list {
                        Self::label_free_expression(&decorator.expression)?;
                    }
                    for parameter in definition.parameters.iter_non_variadic_params() {
                        if let Some(default) = &parameter.default {
                            Self::label_free_expression(default)?;
                        }
                        if let Some(annotation) = &parameter.parameter.annotation {
                            Self::label_free_expression(annotation)?;
                        }
                    }
                    for parameter in [
                        definition.parameters.vararg.as_deref(),
                        definition.parameters.kwarg.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if let Some(annotation) = &parameter.annotation {
                            Self::label_free_expression(annotation)?;
                        }
                    }
                    if let Some(returns) = &definition.returns {
                        Self::label_free_expression(returns)?;
                    }
                    self.instruction(); // MAKE_FUNCTION / decorators / store
                }
                Stmt::ClassDef(definition) => {
                    if definition.type_params.is_some() {
                        return None;
                    }
                    for decorator in &definition.decorator_list {
                        Self::label_free_expression(&decorator.expression)?;
                    }
                    if let Some(arguments) = &definition.arguments {
                        for argument in &arguments.args {
                            Self::label_free_expression(argument)?;
                        }
                        for keyword in &arguments.keywords {
                            Self::label_free_expression(&keyword.value)?;
                        }
                    }
                    self.instruction(); // build-class call / decorators / store
                }
                Stmt::Pass(_) => self.instruction(),
                Stmt::Global(_) | Stmt::Nonlocal(_) => {}
                Stmt::If(statement) => {
                    // The initial subset excludes flattened elif chains and
                    // constant pruning. Each accepted AST node is visited once,
                    // so its range is a unique original codegen occurrence.
                    if !runtime_condition(&statement.test)
                        || statement.elif_else_clauses.len() > 1
                        || statement
                            .elif_else_clauses
                            .first()
                            .is_some_and(|clause| clause.test.is_some())
                    {
                        return None;
                    }
                    let end = self.allocate(NativeRole::IfEnd(statement.range));
                    let otherwise = statement
                        .elif_else_clauses
                        .first()
                        .map(|_| self.allocate(NativeRole::IfElse(statement.range)));
                    self.condition(&statement.test, false)?;
                    self.suite(&statement.body)?;
                    if let Some(otherwise) = otherwise {
                        self.instruction(); // original unconditional join jump
                        self.mark(otherwise);
                        self.suite(&statement.elif_else_clauses[0].body)?;
                    }
                    self.mark(end);
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// Mirrors `compiler_with/compiler_async_with` before any CFG optimization.
    fn manager(&mut self, statement: &ruff_python_ast::StmtWith) -> Option<()> {
        self.manager_items(&statement.items, &statement.body, statement.is_async)
    }

    /// Visits each item recursively, matching the native compiler's `pos` order.
    fn manager_items(
        &mut self,
        items: &[ruff_python_ast::WithItem],
        body: &[Stmt],
        is_async: bool,
    ) -> Option<()> {
        let (item, remaining) = items.split_first()?;
        let range = item.context_expr.range();
        // These four allocations precede evaluating the context expression.
        let block = self.allocate(NativeRole::WithBlock(range));
        let final_ = self.allocate(NativeRole::WithFinal(range));
        let exit = self.allocate(NativeRole::WithExit(range));
        let cleanup = self.allocate(NativeRole::WithCleanup(range));
        self.expression(&item.context_expr)?;
        self.instruction(); // BEFORE_WITH / BEFORE_ASYNC_WITH
        if is_async {
            self.manager_await(range, WithAwait::Enter);
        }
        self.instruction(); // original SETUP_WITH precedes the body label
        self.mark(block);
        if let Some(target) = &item.optional_vars {
            self.store_target(target)?;
        } else {
            self.instruction(); // discard the enter result
        }
        self.manager_depth += 1;
        if remaining.is_empty() {
            self.suite(body)?;
        } else {
            self.manager_items(remaining, body, is_async)?;
        }
        self.manager_depth -= 1;
        self.instruction(); // original POP_BLOCK and normal exit call
        if is_async {
            self.manager_await(range, WithAwait::NormalExit);
        }
        self.instruction(); // POP_TOP / JUMP to the outer exit
        self.mark(final_);
        self.instruction(); // SETUP_CLEANUP / PUSH_EXC_INFO / WITH_EXCEPT_START
        if is_async {
            self.manager_await(range, WithAwait::ExceptionalExit);
        }
        let suppress = self.allocate(NativeRole::WithSuppress(range));
        self.instruction(); // conditional suppression edge / RERAISE
        self.mark(suppress);
        self.instruction(); // suppression POP sequence, including POP_BLOCK
        let helper_exit = self.allocate(NativeRole::WithHelperExit(range));
        self.instruction(); // the helper's JUMP to its own exit label
        self.mark(cleanup);
        self.instruction(); // COPY / POP_EXCEPT / RERAISE
        self.mark(helper_exit);
        // compiler_with_except_finish returns with no intervening instruction.
        // offset2lbl therefore gives both exits the later helper-exit ordinal.
        self.mark(exit);
        Some(())
    }

    fn manager_await(&mut self, range: TextRange, phase: WithAwait) {
        self.instruction(); // GET_AWAITABLE / LOAD_CONST before SEND
        let [send_role, fail_role, exit_role] = phase.roles(range);
        let send = self.allocate(send_role);
        let fail = self.allocate(fail_role);
        let exit = self.allocate(exit_role);
        self.mark(send);
        self.instruction(); // SEND / SETUP_FINALLY / YIELD / POP_BLOCK / RESUME / JUMP
        self.mark(fail);
        self.instruction(); // CLEANUP_THROW
        self.mark(exit);
        self.instruction(); // END_SEND keeps the following setup/body label distinct
    }

    /// Checks evaluation outside a child scope without visiting its body.
    fn label_free_expression(expression: &Expr) -> Option<()> {
        let mut expression_plan = Self::default();
        expression_plan.expression(expression)?;
        expression_plan.labels.is_empty().then_some(())
    }

    fn condition(&mut self, expression: &Expr, jump_on: bool) -> Option<()> {
        match expression {
            Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => {
                self.condition(&unary.operand, !jump_on)?;
            }
            Expr::BoolOp(boolean) => {
                let short_condition = boolean.op == BoolOp::Or;
                let join = (short_condition != jump_on)
                    .then(|| self.allocate(NativeRole::ConditionBoolEnd(boolean.range)));
                let (last, leading) = boolean.values.split_last()?;
                for value in leading {
                    self.condition(value, short_condition)?;
                }
                self.condition(last, jump_on)?;
                if let Some(join) = join {
                    self.mark(join);
                }
            }
            Expr::If(conditional) => {
                let end = self.allocate(NativeRole::ConditionIfEnd(conditional.range));
                let otherwise = self.allocate(NativeRole::ConditionIfElse(conditional.range));
                self.condition(&conditional.test, false)?;
                self.condition(&conditional.body, jump_on)?;
                self.instruction(); // original join jump
                self.mark(otherwise);
                self.condition(&conditional.orelse, jump_on)?;
                self.mark(end);
            }
            _ => {
                self.expression(expression)?;
                self.instruction(); // original conditional jump
            }
        }
        Some(())
    }

    fn store_target(&mut self, target: &Expr) -> Option<()> {
        match target {
            Expr::Name(_) => self.instruction(),
            Expr::Starred(starred) => self.store_target(&starred.value)?,
            Expr::Tuple(tuple) => {
                self.instruction(); // original unpack before target evaluation
                for element in &tuple.elts {
                    self.store_target(element)?;
                }
            }
            Expr::List(list) => {
                self.instruction();
                for element in &list.elts {
                    self.store_target(element)?;
                }
            }
            Expr::Attribute(attribute) => {
                self.expression(&attribute.value)?;
                self.instruction();
            }
            Expr::Subscript(subscript) => {
                self.expression(&subscript.value)?;
                self.expression(&subscript.slice)?;
                self.instruction();
            }
            _ => return None,
        }
        Some(())
    }

    fn interpolated_elements(
        &mut self,
        elements: &ruff_python_ast::InterpolatedStringElements,
    ) -> Option<()> {
        for element in elements {
            match element {
                InterpolatedStringElement::Literal(literal) => {
                    if !literal.value.is_empty() {
                        self.instruction();
                    }
                }
                InterpolatedStringElement::Interpolation(interpolation) => {
                    if interpolation.debug_text.is_some() {
                        self.instruction();
                    }
                    self.expression(&interpolation.expression)?;
                    if let Some(spec) = &interpolation.format_spec {
                        self.interpolated_elements(&spec.elements)?;
                        self.instruction(); // format-spec string construction
                    }
                    self.instruction(); // FORMAT_VALUE
                }
            }
        }
        Some(())
    }

    fn expression(&mut self, expression: &Expr) -> Option<()> {
        match expression {
            Expr::Name(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_) => {}
            Expr::If(conditional) => {
                let end = self.allocate(NativeRole::ValueIfEnd(conditional.range));
                let otherwise = self.allocate(NativeRole::ValueIfElse(conditional.range));
                self.condition(&conditional.test, false)?;
                self.expression(&conditional.body)?;
                self.instruction(); // original join jump
                self.mark(otherwise);
                self.expression(&conditional.orelse)?;
                self.mark(end);
                return Some(()); // native emits nothing after the joined value
            }
            Expr::BoolOp(boolean) => {
                let end = self.allocate(NativeRole::ValueBoolEnd(boolean.range));
                let (last, leading) = boolean.values.split_last()?;
                for value in leading {
                    self.expression(value)?;
                    self.instruction(); // COPY / conditional / POP_TOP
                }
                self.expression(last)?;
                self.mark(end);
                // Native emits nothing after this label. A nested trailing
                // Boolean end can therefore alias its parent's end exactly.
                return Some(());
            }
            Expr::Generator(generator) => {
                // compiler_comprehension compiles this child in a separate code
                // unit, then emits its closure before visiting only the outer
                // iterable here. GET_ITER/GET_AITER and CALL follow it; even an
                // async generator does not synthesize an await in this scope.
                // Builtin generator-call inlining is disabled on legacy targets.
                let outermost = generator.generators.first()?;
                self.instruction(); // LOAD_CONST / MAKE_FUNCTION before iterable
                self.expression(&outermost.iter)?;
            }
            Expr::Lambda(lambda) => {
                // Defaults belong to this scope; the body and its labels do not.
                // Keep defaults label-free until their evaluation order is
                // modelled explicitly, as for the admitted named definitions.
                if let Some(parameters) = &lambda.parameters {
                    for parameter in parameters.iter_non_variadic_params() {
                        if let Some(default) = &parameter.default {
                            Self::label_free_expression(default)?;
                        }
                    }
                }
            }
            Expr::Attribute(attribute) => self.expression(&attribute.value)?,
            Expr::Named(named) => {
                self.expression(&named.value)?;
                self.instruction(); // COPY before the store
                self.store_target(&named.target)?;
            }
            Expr::FString(fstring) => {
                for part in &fstring.value {
                    match part {
                        FStringPart::Literal(literal) => {
                            if !literal.value.is_empty() {
                                self.instruction();
                            }
                        }
                        FStringPart::FString(part) => self.interpolated_elements(&part.elements)?,
                    }
                }
            }
            Expr::Subscript(subscript) => {
                self.expression(&subscript.value)?;
                self.expression(&subscript.slice)?;
            }
            Expr::Slice(slice) => {
                for value in [&slice.lower, &slice.upper, &slice.step]
                    .into_iter()
                    .flatten()
                {
                    self.expression(value)?;
                }
            }
            Expr::UnaryOp(unary) => self.expression(&unary.operand)?,
            Expr::BinOp(binary) => {
                self.expression(&binary.left)?;
                self.expression(&binary.right)?;
            }
            Expr::Compare(comparison) if comparison.ops.len() == 1 => {
                self.expression(&comparison.left)?;
                self.expression(&comparison.comparators[0])?;
            }
            Expr::Call(call) => {
                self.expression(&call.func)?;
                for argument in &call.arguments.args {
                    self.expression(argument)?;
                }
                for keyword in &call.arguments.keywords {
                    self.expression(&keyword.value)?;
                }
            }
            Expr::Starred(starred) => self.expression(&starred.value)?,
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.expression(element)?;
                }
            }
            Expr::List(list) => {
                for element in &list.elts {
                    self.expression(element)?;
                }
            }
            Expr::Set(set) => {
                for element in &set.elts {
                    self.expression(element)?;
                }
            }
            Expr::Dict(dict) => {
                for item in &dict.items {
                    if let Some(key) = &item.key {
                        self.expression(key)?;
                    }
                    self.expression(&item.value)?;
                }
            }
            Expr::Await(awaited) => {
                self.expression(&awaited.value)?;
                self.instruction(); // GET_AWAITABLE / LOAD_CONST before send
                let send = self.allocate(NativeRole::AwaitSend(awaited.range));
                let fail = self.allocate(NativeRole::AwaitFail(awaited.range));
                let exit = self.allocate(NativeRole::AwaitExit(awaited.range));
                self.mark(send);
                self.instruction(); // SEND / SETUP_FINALLY / YIELD / POP_BLOCK / RESUME / JUMP
                self.mark(fail);
                self.instruction(); // CLEANUP_THROW
                self.mark(exit);
            }
            _ => return None,
        }
        // Joined values return above because their end mark has no following
        // instruction. Other accepted expressions emit after any marks they bind
        // (END_SEND for Await), preserving original alias equality.
        self.instruction();
        Some(())
    }
}

fn runtime_condition(expression: &Expr) -> bool {
    match expression {
        Expr::Name(name) => name.id.as_str() != "__debug__",
        Expr::Attribute(_)
        | Expr::Subscript(_)
        | Expr::Call(_)
        | Expr::Await(_)
        | Expr::Compare(_)
        | Expr::BoolOp(_)
        | Expr::If(_) => true,
        Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => runtime_condition(&unary.operand),
        _ => false,
    }
}

impl Compiler<'_> {
    /// Consumes the exact, unreferenced body label of an admitted Name-target For.
    pub(super) fn consume_unreferenced_native_for_body(&mut self, range: TextRange) {
        let Some(plan) = self.native_label_plan.as_mut() else {
            return;
        };
        let body = plan.remaining.remove(&NativeRole::ForBody(range));
        let cleanup = plan.remaining.get(&NativeRole::ForCleanup(range)).copied();
        // compiler_for counts and marks Body, but no jump or SETUP targets it.
        // Its Name store prevents aliases. The later Cleanup remains live via
        // FOR_ITER and real END_FOR through all native max-label allocation
        // phases, so omitting this smaller physical binding preserves that max.
        // Keep Serc's existing NOP, block boundaries and owner proofs intact.
        if !matches!((body, cleanup), (Some(body), Some(cleanup)) if body < cleanup) {
            plan.valid = false;
        }
    }

    pub(super) fn bind_native_source_label(&mut self, role: NativeRole, physical: Label) {
        let Some(plan) = self.native_label_plan.as_mut() else {
            return;
        };
        if let Some(native) = plan.remaining.remove(&role) {
            self.assembler.bind_legacy_native_label(physical, native);
        } else {
            plan.valid = false;
        }
    }

    pub(super) fn certify_native_source_labels(&mut self) {
        if let Some(plan) = self.native_label_plan.take() {
            if plan.valid && plan.remaining.is_empty() {
                self.assembler.certify_legacy_native_labels();
            } else {
                self.assembler.invalidate_legacy_native_labels();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeLabelPlan, NativeRole};
    use ruff_python_ast::Stmt;
    use ruff_python_parser::parse_module;

    #[test]
    fn native_source_labels_count_allocations_and_resolve_original_aliases() {
        let parsed = parse_module("async def f(flag):\n    if flag:\n        if flag:\n            pass\n    await action()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        let [Stmt::If(outer), Stmt::Expr(await_statement)] = function.body.as_slice() else {
            panic!()
        };
        let [Stmt::If(inner)] = outer.body.as_slice() else {
            panic!()
        };
        let ruff_python_ast::Expr::Await(awaited) = await_statement.value.as_ref() else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::FunctionStart], 1);
        // The outer end was allocated second, but both nested ends are marked
        // after the same Pass; native offset2lbl retains the later label 3.
        assert_eq!(plan.remaining[&NativeRole::IfEnd(outer.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::IfEnd(inner.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(awaited.range)], 4);
        assert_eq!(plan.remaining[&NativeRole::AwaitFail(awaited.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::AwaitExit(awaited.range)], 6);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 7);
    }

    #[test]
    fn annotation_only_names_leave_original_label_aliases_adjacent() {
        let parsed = parse_module("async def f(flag):\n    if flag:\n        if flag:\n            pass\n        value: list[int]\n    await action()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        let Stmt::If(outer) = &function.body[0] else {
            panic!()
        };
        let Stmt::If(inner) = &outer.body[0] else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::IfEnd(outer.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::IfEnd(inner.range)], 3);
    }

    #[test]
    fn boolean_context_controls_allocation_and_nested_end_aliases() {
        let parsed = parse_module("async def f(a, b, c):\n    result = a or (b and c)\n    if a or b:\n        pass\n    assert a and b\n    await action()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        let Stmt::Assign(assignment) = &function.body[0] else {
            panic!()
        };
        let ruff_python_ast::Expr::BoolOp(outer) = assignment.value.as_ref() else {
            panic!()
        };
        let ruff_python_ast::Expr::BoolOp(inner) = &outer.values[1] else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::ValueBoolEnd(outer.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::ValueBoolEnd(inner.range)], 3);
        let Stmt::If(conditional) = &function.body[1] else {
            panic!()
        };
        let ruff_python_ast::Expr::BoolOp(test) = conditional.test.as_ref() else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::IfEnd(conditional.range)], 4);
        assert_eq!(plan.remaining[&NativeRole::ConditionBoolEnd(test.range)], 5);
        let Stmt::Assert(assertion) = &function.body[2] else {
            panic!()
        };
        let ruff_python_ast::Expr::BoolOp(test) = assertion.test.as_ref() else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::AssertEnd(assertion.range)], 6);
        assert_eq!(plan.remaining[&NativeRole::ConditionBoolEnd(test.range)], 7);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 11);
    }

    #[test]
    fn child_code_units_do_not_allocate_parent_labels() {
        let parsed = parse_module("async def f():\n    def child(value: Kind = default()):\n        while condition():\n            work()\n    class C(Base):\n        def method(self):\n            try:\n                work()\n            except Error:\n                pass\n    await action()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 5);
        let parsed = parse_module("async def f():\n    def child(value=left or right):\n        pass\n    await action()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        assert!(NativeLabelPlan::function(&function.body, true).is_none());
    }

    #[test]
    fn generator_and_lambda_children_do_not_allocate_parent_labels() {
        for source in [
            "async def f(xs):\n    result = (await action(x) for x in xs if x)\n    await done()\n",
            "async def f(xs):\n    result = (x async for x in xs if x)\n    await done()\n",
            "async def f(xs):\n    result = lambda value=default(), *, other=second(): [x for x in xs if x]\n    await done()\n",
            "async def f(xs):\n    result = all(x for x in xs if x)\n    await done()\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            let plan = NativeLabelPlan::function(&function.body, true).unwrap();
            assert_eq!(plan.remaining.len(), 5, "{source}");
            assert_eq!(
                plan.remaining[&NativeRole::StopIterationHandler],
                5,
                "{source}"
            );
        }
    }

    #[test]
    fn generator_outer_await_belongs_to_the_parent_scope() {
        let parsed = parse_module(
            "async def f():\n    result = (x for x in await items())\n    await done()\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(plan.remaining.len(), 8);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 8);
        for source in [
            "async def f():\n    result = lambda value=left or right: value\n    await done()\n",
            "async def f():\n    result = lambda value=await default(): value\n    await done()\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(NativeLabelPlan::function(&function.body, true).is_none());
        }
    }

    #[test]
    fn for_labels_are_reserved_before_awaiting_the_iterable() {
        let parsed = parse_module("async def f():\n    for item in await items():\n        await action(item)\n    else:\n        pass\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::For(statement)] = function.body.as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(plan.remaining[&NativeRole::ForStart(statement.range)], 2);
        assert_eq!(plan.remaining[&NativeRole::ForBody(statement.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::ForCleanup(statement.range)], 4);
        assert_eq!(plan.remaining[&NativeRole::ForEnd(statement.range)], 5);
        let ruff_python_ast::Expr::Await(iterable) = statement.iter.as_ref() else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(iterable.range)], 6);
        let [Stmt::Expr(body)] = statement.body.as_slice() else {
            panic!()
        };
        let ruff_python_ast::Expr::Await(body) = body.value.as_ref() else {
            panic!()
        };
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(body.range)], 9);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 12);
    }

    #[test]
    fn unsupported_for_routes_reject_the_whole_scope() {
        for source in [
            "async def f(xs):\n    async for item in xs:\n        pass\n",
            "def f(xs):\n    for a, b in xs:\n        pass\n",
            "def f(xs):\n    for item in xs:\n        break\n",
            "def f(xs):\n    for item in xs:\n        continue\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, function.is_async).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn conditional_values_reserve_end_before_else_and_visit_both_arms() {
        let parsed = parse_module(
            "async def f(flag):\n    result = await first() if flag else await second()\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        let Stmt::Assign(assignment) = &function.body[0] else {
            panic!()
        };
        let ruff_python_ast::Expr::If(conditional) = assignment.value.as_ref() else {
            panic!()
        };
        let ruff_python_ast::Expr::Await(first) = conditional.body.as_ref() else {
            panic!()
        };
        let ruff_python_ast::Expr::Await(second) = conditional.orelse.as_ref() else {
            panic!()
        };
        assert_eq!(
            plan.remaining[&NativeRole::ValueIfEnd(conditional.range)],
            2
        );
        assert_eq!(
            plan.remaining[&NativeRole::ValueIfElse(conditional.range)],
            3
        );
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(first.range)], 4);
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(second.range)], 7);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 10);
    }

    #[test]
    fn unsupported_dead_source_invalidates_the_entire_native_plan() {
        let parsed = parse_module("async def f():\n    await action()\n    return\n    while condition():\n        pass\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        assert!(NativeLabelPlan::function(&function.body, true).is_none());
    }
}

#[cfg(test)]
mod manager_tests {
    use ruff_python_ast::Stmt;
    use ruff_python_parser::parse_module;
    use ruff_text_size::Ranged;

    use super::{NativeLabelPlan, NativeRole, WithAwait};

    #[test]
    fn synchronous_manager_reserves_late_helper_exit_alias() {
        let parsed = parse_module("def f():\n    with manager():\n        pass\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::With(manager)] = function.body.as_slice() else {
            panic!()
        };
        let range = manager.items[0].context_expr.range();
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        for (role, ordinal) in [
            (NativeRole::WithBlock(range), 2),
            (NativeRole::WithFinal(range), 3),
            (NativeRole::WithExit(range), 7),
            (NativeRole::WithCleanup(range), 5),
            (NativeRole::WithSuppress(range), 6),
            (NativeRole::WithHelperExit(range), 7),
        ] {
            assert_eq!(plan.remaining[&role], ordinal);
        }
    }

    #[test]
    fn async_manager_allocates_its_three_awaits_in_native_order() {
        let parsed =
            parse_module("async def f():\n    async with manager():\n        pass\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::With(manager)] = function.body.as_slice() else {
            panic!()
        };
        let range = manager.items[0].context_expr.range();
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        for (phase, first) in [
            (WithAwait::Enter, 6),
            (WithAwait::NormalExit, 9),
            (WithAwait::ExceptionalExit, 12),
        ] {
            for (role, ordinal) in phase.roles(range).into_iter().zip(first..) {
                assert_eq!(plan.remaining[&role], ordinal);
            }
        }
        assert_eq!(plan.remaining[&NativeRole::WithBlock(range)], 2);
        assert_eq!(plan.remaining[&NativeRole::WithSuppress(range)], 15);
        assert_eq!(plan.remaining[&NativeRole::WithExit(range)], 16);
        assert_eq!(plan.remaining[&NativeRole::WithHelperExit(range)], 16);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 17);
    }

    #[test]
    fn sequential_managers_keep_distinct_occurrences() {
        let parsed = parse_module(
            "def f():\n    with first():\n        pass\n    with second():\n        pass\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::With(first), Stmt::With(second)] = function.body.as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        assert_eq!(
            plan.remaining[&NativeRole::WithExit(first.items[0].context_expr.range())],
            7
        );
        assert_eq!(
            plan.remaining[&NativeRole::WithBlock(second.items[0].context_expr.range())],
            8
        );
        assert_eq!(
            plan.remaining[&NativeRole::WithExit(second.items[0].context_expr.range())],
            13
        );
    }

    #[test]
    fn nested_and_multiple_managers_preserve_recursive_allocation_order() {
        for source in [
            "def f():\n    with first(), second():\n        pass\n",
            "def f():\n    with first():\n        with second():\n            pass\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            let [Stmt::With(outer)] = function.body.as_slice() else {
                panic!()
            };
            let first = outer.items[0].context_expr.range();
            let second = if outer.items.len() == 2 {
                outer.items[1].context_expr.range()
            } else {
                let [Stmt::With(inner)] = outer.body.as_slice() else {
                    panic!()
                };
                inner.items[0].context_expr.range()
            };
            let plan = NativeLabelPlan::function(&function.body, false).unwrap();
            assert_eq!(plan.remaining[&NativeRole::WithBlock(first)], 2);
            assert_eq!(plan.remaining[&NativeRole::WithBlock(second)], 6);
            assert_eq!(plan.remaining[&NativeRole::WithExit(second)], 11);
            assert_eq!(plan.remaining[&NativeRole::WithExit(first)], 13);
        }
    }

    #[test]
    fn multiple_async_managers_complete_inner_awaits_before_outer_cleanup() {
        let parsed =
            parse_module("async def f():\n    async with first(), second():\n        pass\n")
                .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::With(manager)] = function.body.as_slice() else {
            panic!()
        };
        let first = manager.items[0].context_expr.range();
        let second = manager.items[1].context_expr.range();
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(
            plan.remaining[&NativeRole::WithAwaitSend(first, WithAwait::Enter)],
            6
        );
        assert_eq!(
            plan.remaining[&NativeRole::WithAwaitSend(second, WithAwait::Enter)],
            13
        );
        assert_eq!(
            plan.remaining[&NativeRole::WithAwaitSend(second, WithAwait::NormalExit)],
            16
        );
        assert_eq!(
            plan.remaining[&NativeRole::WithAwaitSend(first, WithAwait::NormalExit)],
            24
        );
        assert_eq!(plan.remaining[&NativeRole::WithExit(second)], 23);
        assert_eq!(plan.remaining[&NativeRole::WithExit(first)], 31);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 32);
    }

    #[test]
    fn unsupported_manager_unwinds_reject_the_whole_plan() {
        for source in [
            "def f():\n    with manager():\n        return 1\n",
            "def f():\n    with first(), second():\n        return 1\n",
            "def f():\n    with first():\n        with second():\n            return 1\n",
            "def f():\n    with manager():\n        try:\n            work()\n        finally:\n            finish()\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(NativeLabelPlan::function(&function.body, false).is_none());
        }
    }
}
