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
    TryBody(TextRange),
    TryExcept(TextRange),
    TryEnd(TextRange),
    TryCleanup(TextRange),
    HandlerNext(TextRange),
    HandlerCleanupBody(TextRange),
    HandlerCleanupEnd(TextRange),
    ValueBoolEnd(TextRange),
    ValueCompareCleanup(TextRange),
    ValueCompareEnd(TextRange),
    ConditionCompareCleanup(TextRange),
    ConditionCompareEnd(TextRange),
    ConditionBoolEnd(TextRange),
    ValueIfEnd(TextRange),
    ValueIfElse(TextRange),
    ConditionIfEnd(TextRange),
    ConditionIfElse(TextRange),
    ForStart(TextRange),
    ForBody(TextRange),
    ForCleanup(TextRange),
    ForEnd(TextRange),
    ComprehensionCleanup(TextRange),
    ComprehensionEnd(TextRange),
    ComprehensionStart(TextRange),
    ComprehensionIfCleanup(TextRange),
    ComprehensionAnchor(TextRange),
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
    handler_depth: usize,
    for_manager_depths: Vec<usize>,
    inside_comprehension_body: bool,
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
                Stmt::AugAssign(assignment) => {
                    match assignment.target.as_ref() {
                        Expr::Name(_) => {}
                        target @ (Expr::Attribute(_) | Expr::Subscript(_)) => {
                            Self::label_free_expression(target)?;
                        }
                        _ => return None,
                    }
                    // The original target evaluation and COPY/load prefix are
                    // nonempty and label-free. RHS labels retain their own order.
                    self.instruction();
                    self.expression(&assignment.value)?;
                    self.instruction();
                }
                Stmt::Expr(expression) => {
                    self.expression(&expression.value)?;
                    self.instruction(); // POP_TOP, or the constant statement's NOP
                }
                Stmt::Return(statement) => {
                    if self.manager_depth != 0 || self.handler_depth != 0 {
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
                    if !statement.is_async && Self::name_unpack_target(&statement.target) =>
                {
                    // compiler_for reserves all four labels before visiting the
                    // iterable. Its body mark follows the original target NOP.
                    let start = self.allocate(NativeRole::ForStart(statement.range));
                    let body = self.allocate(NativeRole::ForBody(statement.range));
                    let cleanup = self.allocate(NativeRole::ForCleanup(statement.range));
                    let end = self.allocate(NativeRole::ForEnd(statement.range));
                    // Native pushes FOR_LOOP before evaluating the iterable. A
                    // continue can reuse its start only without crossing a manager.
                    self.for_manager_depths.push(self.manager_depth);
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
                    self.for_manager_depths.pop();
                    self.suite(&statement.orelse)?;
                    self.mark(end);
                }
                Stmt::Break(_) if self.for_manager_depths.last() == Some(&self.manager_depth) => {
                    self.instruction(); // original break NOP
                    self.instruction(); // POP_TOP unwinds the nearest For iterator
                    self.instruction(); // JUMP to the existing ForEnd, after else
                }
                Stmt::Continue(_)
                    if self.for_manager_depths.last() == Some(&self.manager_depth) =>
                {
                    self.instruction(); // original continue NOP
                    self.instruction(); // JUMP to the existing nearest ForStart
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
                Stmt::Try(statement) => self.try_except(statement)?,
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
                    self.if_statement(
                        statement.range,
                        &statement.test,
                        &statement.body,
                        &statement.elif_else_clauses,
                    )?;
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// Mirrors typed `compiler_try_except` handlers with their original shared join.
    fn try_except(&mut self, statement: &ruff_python_ast::StmtTry) -> Option<()> {
        if statement.is_star
            || statement.handlers.is_empty()
            || !statement.finalbody.is_empty()
            || !statement.orelse.is_empty()
        {
            return None;
        }
        // Native reserves all four before visiting the protected suite. The
        // setup instruction separates Body from any preceding original mark.
        let body = self.allocate(NativeRole::TryBody(statement.range));
        let except = self.allocate(NativeRole::TryExcept(statement.range));
        let end = self.allocate(NativeRole::TryEnd(statement.range));
        let cleanup = self.allocate(NativeRole::TryCleanup(statement.range));
        self.instruction(); // SETUP_FINALLY
        self.mark(body);
        self.suite(&statement.body)?;
        self.instruction(); // original POP_BLOCK and JUMP End, even after an exit
        self.mark(except);
        self.instruction(); // SETUP_CLEANUP and PUSH_EXC_INFO
        for handler in &statement.handlers {
            let handler = handler.as_except_handler()?;
            // Bare handlers omit a dead RERAISE boundary in Serc. They need
            // a separate unreferenced-label proof before sharing this route.
            let exception_type = handler.type_.as_deref()?;
            Self::label_free_expression(exception_type)?;
            let next = self.allocate(NativeRole::HandlerNext(handler.range));
            self.expression(exception_type)?;
            self.instruction(); // CHECK_EXC_MATCH and POP_JUMP_IF_FALSE
            let name_cleanup = handler
                .name
                .as_ref()
                .map(|_| self.allocate(NativeRole::HandlerCleanupEnd(handler.range)));
            let handler_body = self.allocate(NativeRole::HandlerCleanupBody(handler.range));
            self.instruction(); // STORE / SETUP_CLEANUP, or unnamed POP_TOP
            self.mark(handler_body);
            self.handler_depth += 1;
            self.suite(&handler.body)?;
            self.handler_depth -= 1;
            // Ordinary try/handler Break and Continue unwinding allocates no
            // labels. Keep these original cleanup/join boundaries even when
            // an abrupt exit makes the normal records unreachable.
            self.instruction(); // POP_BLOCK / POP_EXCEPT / optional clear / JUMP End
            if let Some(name_cleanup) = name_cleanup {
                self.mark(name_cleanup);
                self.instruction(); // LOAD_CONST / STORE / DELETE / RERAISE
            }
            self.mark(next);
        }
        self.instruction(); // final typed mismatch RERAISE
        self.mark(cleanup);
        self.instruction(); // COPY / POP_EXCEPT / RERAISE
        self.mark(end);
        Some(())
    }

    /// Mirrors `compiler_if`, including CPython's nested If nodes for each elif.
    fn if_statement(
        &mut self,
        range: TextRange,
        test: &Expr,
        body: &[Stmt],
        clauses: &[ruff_python_ast::ElifElseClause],
    ) -> Option<()> {
        if !runtime_condition(test) {
            return None;
        }
        let end = self.allocate(NativeRole::IfEnd(range));
        let otherwise = clauses
            .first()
            .map(|_| self.allocate(NativeRole::IfElse(range)));
        self.condition(test, false)?;
        self.suite(body)?;
        if let Some(otherwise) = otherwise {
            // Native emits this original join even after a terminating body.
            // The next elif allocates its labels only after this boundary.
            self.instruction();
            self.mark(otherwise);
            let (clause, remaining) = clauses.split_first()?;
            if let Some(test) = &clause.test {
                self.if_statement(clause.range, test, &clause.body, remaining)?;
            } else {
                if !remaining.is_empty() {
                    return None;
                }
                self.suite(&clause.body)?;
            }
        }
        // Every nested elif end marks the same original boundary. A later
        // body or declaration-only else label may be their canonical alias.
        self.mark(end);
        Some(())
    }

    /// Mirrors an inlined synchronous comprehension with one shared save/restore.
    fn comprehension(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        key: Option<&Expr>,
        value: &Expr,
    ) -> Option<()> {
        let first = generators.first()?;
        if generators
            .iter()
            .any(|generator| generator.is_async || !Self::name_unpack_target(&generator.target))
            || generators.iter().skip(1).any(|generator| {
                // Native's inner singleton shortcut reserves three labels but
                // leaves Start and Anchor unmarked. Reject the source shape,
                // including folded variants, until discarded roles are proved.
                matches!(&generator.iter, Expr::List(list) if list.elts.len() == 1)
                    || matches!(&generator.iter, Expr::Tuple(tuple) if tuple.elts.len() == 1)
            })
        {
            return None;
        }
        // An inner comprehension's outer iterable still belongs to the enclosing
        // body. Preserve that suspension restriction across recursive scopes.
        let previous_body = self.inside_comprehension_body;
        self.expression(&first.iter)?;
        self.instruction(); // GET_ITER / LOAD_FAST_AND_CLEAR / SWAP
        self.inside_comprehension_body = true;
        let range = first.iter.range();
        // Nonempty Name targets guarantee a saved local, including cell/free
        // names and repeated targets. All clauses share this cleanup pair.
        let cleanup = self.allocate(NativeRole::ComprehensionCleanup(range));
        let end = self.allocate(NativeRole::ComprehensionEnd(range));
        self.instruction(); // SETUP_FINALLY / BUILD_LIST, SET or MAP / SWAP
        self.comprehension_generator(generators, 0, key, value)?;
        self.instruction(); // original POP_BLOCK / JUMP to normal restoration
        self.mark(cleanup);
        self.instruction(); // exceptional restoration / RERAISE
        self.mark(end);
        self.instruction(); // normal SWAP / STORE_FAST_MAYBE_NULL
        self.inside_comprehension_body = previous_body;
        Some(())
    }

    /// Reserves each native clause's labels before visiting its inner iterable.
    fn comprehension_generator(
        &mut self,
        generators: &[ruff_python_ast::Comprehension],
        index: usize,
        key: Option<&Expr>,
        value: &Expr,
    ) -> Option<()> {
        let generator = &generators[index];
        let range = generator.iter.range();
        let start = self.allocate(NativeRole::ComprehensionStart(range));
        let if_cleanup = self.allocate(NativeRole::ComprehensionIfCleanup(range));
        let anchor = self.allocate(NativeRole::ComprehensionAnchor(range));
        if index != 0 {
            self.expression(&generator.iter)?;
            self.instruction(); // inner GET_ITER
        }
        self.mark(start);
        self.instruction(); // FOR_ITER
        self.store_target(&generator.target)?;
        for filter in &generator.ifs {
            if !runtime_condition(filter) {
                return None;
            }
            self.condition(filter, false)?;
        }
        if index + 1 < generators.len() {
            self.comprehension_generator(generators, index + 1, key, value)?;
        } else {
            if let Some(key) = key {
                self.expression(key)?;
            }
            self.expression(value)?;
            self.instruction(); // LIST_APPEND / SET_ADD / MAP_ADD
        }
        self.mark(if_cleanup);
        self.instruction(); // original backedge
        self.mark(anchor);
        // This real instruction separates the inner Anchor from its enclosing
        // IfCleanup even when later optimization removes a dead result path.
        self.instruction(); // END_FOR
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
            Expr::Compare(comparison) if comparison.ops.len() > 1 => {
                // compiler_jump_if reserves Cleanup before visiting the left
                // operand, but delays End until after the final conditional.
                let cleanup = self.allocate(NativeRole::ConditionCompareCleanup(comparison.range));
                self.expression(&comparison.left)?;
                let (last, leading) = comparison.comparators.split_last()?;
                for comparator in leading {
                    self.expression(comparator)?;
                    self.instruction(); // SWAP / COPY / COMPARE / conditional
                }
                self.expression(last)?;
                self.instruction(); // final comparison and conditional jump
                let end = self.allocate(NativeRole::ConditionCompareEnd(comparison.range));
                self.instruction(); // jump around failed-comparison cleanup
                self.mark(cleanup);
                self.instruction(); // POP_TOP
                if !jump_on {
                    self.instruction(); // failed comparison jumps to caller's label
                }
                self.mark(end);
            }
            _ => {
                self.expression(expression)?;
                self.instruction(); // original conditional jump
            }
        }
        Some(())
    }

    /// Admits Name stores and nonempty unpacking without target-expression labels.
    fn name_unpack_target(target: &Expr) -> bool {
        match target {
            Expr::Name(_) => true,
            Expr::Tuple(tuple) => {
                !tuple.elts.is_empty() && tuple.elts.iter().all(Self::name_unpack_target)
            }
            Expr::List(list) => {
                !list.elts.is_empty() && list.elts.iter().all(Self::name_unpack_target)
            }
            _ => false,
        }
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
            Expr::ListComp(comprehension) => {
                return self.comprehension(&comprehension.generators, None, &comprehension.elt);
            }
            Expr::SetComp(comprehension) => {
                return self.comprehension(&comprehension.generators, None, &comprehension.elt);
            }
            Expr::DictComp(comprehension) => {
                return self.comprehension(
                    &comprehension.generators,
                    Some(comprehension.key.as_deref()?),
                    &comprehension.value,
                );
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
            Expr::Compare(comparison) if comparison.ops.len() > 1 => {
                // Unlike condition lowering, compiler_compare visits the left
                // operand before reserving its failed-comparison cleanup.
                self.expression(&comparison.left)?;
                let cleanup = self.allocate(NativeRole::ValueCompareCleanup(comparison.range));
                let (last, leading) = comparison.comparators.split_last()?;
                for comparator in leading {
                    self.expression(comparator)?;
                    self.instruction(); // SWAP / COPY / COMPARE / conditional / POP
                }
                self.expression(last)?;
                self.instruction(); // final comparison
                let end = self.allocate(NativeRole::ValueCompareEnd(comparison.range));
                self.instruction(); // jump around failed-comparison cleanup
                self.mark(cleanup);
                self.instruction(); // SWAP / POP_TOP
                self.mark(end);
                return Some(()); // no instruction follows the joined value
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
            Expr::Await(awaited) if !self.inside_comprehension_body => {
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
    /// Consumes the exact, unreferenced body label of an admitted For target.
    pub(super) fn consume_unreferenced_native_for_body(&mut self, range: TextRange) {
        let Some(plan) = self.native_label_plan.as_mut() else {
            return;
        };
        let body = plan.remaining.remove(&NativeRole::ForBody(range));
        let cleanup = plan.remaining.get(&NativeRole::ForCleanup(range)).copied();
        // compiler_for counts and marks Body, but no jump or SETUP targets it.
        // Its Name store or initial UNPACK prevents aliases. Cleanup remains live via
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
    fn elif_allocations_interleave_with_bodies_and_share_the_nested_end() {
        let parsed = parse_module("async def f(a, b, c):\n    if a:\n        await first()\n    elif b:\n        await second()\n    else:\n        if c:\n            pass\n    await done()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::If(outer), Stmt::Expr(after)] = function.body.as_slice() else {
            panic!()
        };
        let [clause, otherwise] = outer.elif_else_clauses.as_slice() else {
            panic!()
        };
        let [Stmt::If(nested)] = otherwise.body.as_slice() else {
            panic!()
        };
        let ruff_python_ast::Expr::Await(after) = after.value.as_ref() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(plan.remaining[&NativeRole::IfElse(outer.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::IfElse(clause.range)], 8);
        for range in [outer.range, clause.range, nested.range] {
            assert_eq!(plan.remaining[&NativeRole::IfEnd(range)], 12);
        }
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(after.range)], 13);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 16);
    }

    #[test]
    fn elif_without_else_reserves_no_final_false_label() {
        let parsed = parse_module("def f(a, b, c):\n    if a:\n        pass\n    elif b:\n        pass\n    elif c:\n        pass\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::If(outer)] = function.body.as_slice() else {
            panic!()
        };
        let [first, last] = outer.elif_else_clauses.as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        for range in [outer.range, first.range, last.range] {
            assert_eq!(plan.remaining[&NativeRole::IfEnd(range)], 6);
        }
        assert_eq!(plan.remaining[&NativeRole::IfElse(outer.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::IfElse(first.range)], 5);
        assert!(!plan.remaining.contains_key(&NativeRole::IfElse(last.range)));
    }

    #[test]
    fn declaration_only_else_can_be_the_canonical_elif_end() {
        let parsed = parse_module("def f(a, b):\n    if a:\n        return\n    elif b:\n        return\n    else:\n        global shared\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::If(outer)] = function.body.as_slice() else {
            panic!()
        };
        let clause = &outer.elif_else_clauses[0];
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        assert_eq!(plan.remaining[&NativeRole::IfEnd(outer.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::IfEnd(clause.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::IfElse(clause.range)], 5);
    }

    #[test]
    fn unsupported_or_constant_elif_invalidates_the_whole_plan() {
        for source in [
            "def f(a):\n    if a:\n        pass\n    elif True:\n        pass\n",
            "def f(a, b):\n    if a:\n        return\n    elif b:\n        return\n        while work():\n            pass\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(NativeLabelPlan::function(&function.body, false).is_none());
        }
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
            "def f(xs):\n    while xs:\n        break\n",
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
    #[test]
    fn unpacked_for_continue_reuses_the_existing_native_loop_labels() {
        let parsed = parse_module("async def f(items, flag):\n    for first, [second, third] in items:\n        if flag:\n            continue\n    await done()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let Stmt::For(loop_) = &function.body[0] else {
            panic!()
        };
        let Stmt::If(conditional) = &loop_.body[0] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(plan.remaining[&NativeRole::ForStart(loop_.range)], 2);
        assert_eq!(plan.remaining[&NativeRole::ForBody(loop_.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::ForCleanup(loop_.range)], 4);
        assert_eq!(plan.remaining[&NativeRole::ForEnd(loop_.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::IfEnd(conditional.range)], 6);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 10);
        assert_eq!(plan.remaining.len(), 10);
    }

    #[test]
    fn for_continue_stops_at_the_nearest_loop_before_manager_cleanup() {
        for source in [
            "def f(items, cm):\n    with cm:\n        for first, second in items:\n            continue\n",
            "def f(items, cm):\n    for first, second in items:\n        with cm:\n            for value in items:\n                continue\n",
            "def f(items):\n    for first, second in items:\n        for value in items:\n            pass\n        else:\n            continue\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_some(),
                "{source}"
            );
        }
        for source in [
            "def f(items, cm):\n    for first, second in items:\n        with cm:\n            continue\n",
            "def f(items):\n    for first, second in items:\n        pass\n    else:\n        continue\n",
            "def f(items):\n    while items:\n        continue\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn for_unpacking_declines_targets_with_other_store_routes() {
        for target in [
            "first, *rest",
            "holder.item",
            "holder[index]",
            "()",
            "first, ()",
        ] {
            let source = format!("def f(items):\n    for {target} in items:\n        pass\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn inlined_comprehension_labels_follow_the_outer_iterable_await() {
        let parsed = parse_module(
            "async def f():\n    values = [item for item in await items()]\n    await done()\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let Stmt::Assign(assignment) = &function.body[0] else {
            panic!()
        };
        let ruff_python_ast::Expr::ListComp(comprehension) = assignment.value.as_ref() else {
            panic!()
        };
        let range = comprehension.generators[0].iter.range();
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        // Native 3.12 reserves the coroutine body first, then the explicit
        // iterable await's three labels, then the five inlined scope/loop labels.
        assert_eq!(plan.remaining[&NativeRole::ComprehensionCleanup(range)], 5);
        assert_eq!(plan.remaining[&NativeRole::ComprehensionEnd(range)], 6);
        assert_eq!(plan.remaining[&NativeRole::ComprehensionStart(range)], 7);
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionIfCleanup(range)],
            8
        );
        assert_eq!(plan.remaining[&NativeRole::ComprehensionAnchor(range)], 9);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 13);
        assert_eq!(plan.remaining.len(), 13);
    }

    #[test]
    fn inlined_comprehension_subset_rejects_additional_control_routes() {
        for expression in [
            "[item async for item in xs]",
            "[await action(item) for item in xs]",
            "[item for item in xs for other in await ys()]",
            "[item for item in xs for other in [value]]",
            "[item for item in xs for other in (value,)]",
            "[item for item in xs for other in [1]]",
            "[item for item in xs for other in [*values]]",
        ] {
            let source = format!("async def f(xs, ys):\n    values = {expression}\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, true).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn comprehension_filters_reuse_the_existing_native_cleanup_label() {
        let parsed = parse_module(
            "async def f(flag, allowed):\n    values = [item for item in await items() if flag if item in allowed if predicate(item)]\n    await done()\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let Stmt::Assign(assignment) = &function.body[0] else {
            panic!()
        };
        let ruff_python_ast::Expr::ListComp(comprehension) = assignment.value.as_ref() else {
            panic!()
        };
        let range = comprehension.generators[0].iter.range();
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        // Iterable await allocates 2..4, then the same five comprehension labels
        // as an unfiltered expression. None of the three filters adds a label.
        assert_eq!(plan.remaining[&NativeRole::ComprehensionCleanup(range)], 5);
        assert_eq!(plan.remaining[&NativeRole::ComprehensionEnd(range)], 6);
        assert_eq!(plan.remaining[&NativeRole::ComprehensionStart(range)], 7);
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionIfCleanup(range)],
            8
        );
        assert_eq!(plan.remaining[&NativeRole::ComprehensionAnchor(range)], 9);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 13);
        assert_eq!(plan.remaining.len(), 13);
    }

    #[test]
    fn comprehension_filters_compose_with_existing_loop_and_manager_roles() {
        for expression in [
            "[item for item in xs if item]",
            "{item for item in xs if item is not None}",
            "{item: value(item) for item in xs if predicate(item, enabled=flag)}",
            "[item for item in xs if item in allowed if predicate(item)]",
            "[item for item in xs for other in ys]",
            "[left or right for item in xs]",
            "{(left if flag else right): item for item in xs}",
            "[item for item in xs if not item]",
            "[item for item in xs if item.enabled if item[index]]",
            "[item for item in xs if item and flag if item or flag]",
            "[item for item in xs if (item if flag else other)]",
            "[item for item in xs if item == (left or right)]",
            "[item for item in xs if predicate(item, enabled=left or right)]",
        ] {
            let source = format!(
                "def f(xs, flag, allowed, cm):\n    with cm:\n        for first, second in xs:\n            values = {expression}\n            if flag:\n                continue\n"
            );
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_some(),
                "{source}"
            );
        }
    }

    #[test]
    fn comprehension_filters_decline_other_branch_and_evaluation_routes() {
        for filter in [
            "True",
            "__debug__",
            "predicate(await value())",
            "await predicate(item)",
        ] {
            let source =
                format!("async def f(xs, ys):\n    values = [item for item in xs if {filter}]\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, true).is_none(),
                "{source}"
            );
        }
    }
    #[test]
    fn augmented_attribute_and_subscript_targets_add_no_native_labels() {
        for target in [
            "obj.value",
            "items[index]",
            "obj.items[index, other.index]",
            "items[start:stop]",
            "items[start:stop:step]",
        ] {
            let source = format!(
                "async def f(flag):\n    if flag:\n        pass\n    {target} += await rhs()\n    await done()\n"
            );
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            let [
                Stmt::If(condition),
                Stmt::AugAssign(assignment),
                Stmt::Expr(after),
            ] = function.body.as_slice()
            else {
                panic!()
            };
            let ruff_python_ast::Expr::Await(value) = assignment.value.as_ref() else {
                panic!()
            };
            let ruff_python_ast::Expr::Await(after) = after.value.as_ref() else {
                panic!()
            };
            let plan = NativeLabelPlan::function(&function.body, true).unwrap();
            assert_eq!(plan.remaining[&NativeRole::IfEnd(condition.range)], 2);
            assert_eq!(plan.remaining[&NativeRole::AwaitSend(value.range)], 3);
            assert_eq!(plan.remaining[&NativeRole::AwaitExit(value.range)], 5);
            assert_eq!(plan.remaining[&NativeRole::AwaitSend(after.range)], 6);
            assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 9);
        }
    }

    #[test]
    fn augmented_targets_keep_label_bearing_evaluation_outside_the_subset() {
        for target in [
            "(await holder()).value",
            "(left or right).value",
            "items[(left if flag else right)]",
            "items[await index()]",
            "[item for item in values][0]",
        ] {
            let source = format!("async def f():\n    {target} += value\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(NativeLabelPlan::function(&function.body, true).is_none());
        }
    }

    #[test]
    fn for_break_and_comprehension_unpacking_keep_existing_native_ordinals() {
        let parsed = parse_module(
            "async def f(items, flag):\n    for first, second in items:\n        if flag:\n            break\n    else:\n        fallback()\n    values = {first: second for first, [second, third] in await source() if keep(first)}\n    await done()\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let Stmt::For(loop_) = &function.body[0] else {
            panic!()
        };
        let Stmt::If(conditional) = &loop_.body[0] else {
            panic!()
        };
        let Stmt::Assign(assignment) = &function.body[1] else {
            panic!()
        };
        let ruff_python_ast::Expr::DictComp(comprehension) = assignment.value.as_ref() else {
            panic!()
        };
        let range = comprehension.generators[0].iter.range();
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        // Break targets ForEnd and allocates no labels. All three saved names
        // share the same comprehension cleanup/end pair after iterable Await.
        for (role, ordinal) in [
            (NativeRole::ForStart(loop_.range), 2),
            (NativeRole::ForBody(loop_.range), 3),
            (NativeRole::ForCleanup(loop_.range), 4),
            (NativeRole::ForEnd(loop_.range), 5),
            (NativeRole::IfEnd(conditional.range), 6),
            (NativeRole::ComprehensionCleanup(range), 10),
            (NativeRole::ComprehensionEnd(range), 11),
            (NativeRole::ComprehensionStart(range), 12),
            (NativeRole::ComprehensionIfCleanup(range), 13),
            (NativeRole::ComprehensionAnchor(range), 14),
            (NativeRole::StopIterationHandler, 18),
        ] {
            assert_eq!(plan.remaining[&role], ordinal);
        }
        assert_eq!(plan.remaining.len(), 18);
    }

    #[test]
    fn for_break_stops_at_the_nearest_loop_before_manager_cleanup() {
        for source in [
            "def f(items, cm):\n    with cm:\n        for item in items:\n            break\n",
            "def f(items, cm):\n    for item in items:\n        with cm:\n            for other in items:\n                break\n",
            "def f(items):\n    for item in items:\n        for other in items:\n            pass\n        else:\n            break\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_some(),
                "{source}"
            );
        }
        for source in [
            "def f(items, cm):\n    for item in items:\n        with cm:\n            break\n",
            "def f(items):\n    for item in items:\n        pass\n    else:\n        break\n",
            "def f(items):\n    while items:\n        break\n",
            "async def f(items):\n    async for item in items:\n        break\n",
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
    fn comprehension_unpacking_keeps_a_nonempty_native_save_set() {
        for target in [
            "first, second",
            "[first, second]",
            "first, [second, third]",
            "first, first",
        ] {
            let source = format!("def f(items):\n    values = [first for {target} in items]\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_some(),
                "{source}"
            );
        }
        for target in [
            "()",
            "first, ()",
            "first, *rest",
            "holder.item",
            "holder[index]",
        ] {
            let source = format!("def f(items):\n    values = [value for {target} in items]\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_none(),
                "{source}"
            );
        }
    }
}

#[cfg(test)]
mod ordinary_except_tests {
    use super::{NativeLabelPlan, NativeRole};
    use ruff_python_ast::{Expr, Stmt};
    use ruff_python_parser::parse_module;

    #[test]
    fn typed_handler_labels_interleave_with_authored_awaits() {
        let parsed = parse_module("async def f(flag):\n    try:\n        await action()\n    except First as error:\n        if flag:\n            await recover(error)\n    except (Second, Third):\n        pass\n    await done()\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Try(statement), Stmt::Expr(after)] = function.body.as_slice() else {
            panic!()
        };
        let first = statement.handlers[0].as_except_handler().unwrap();
        let second = statement.handlers[1].as_except_handler().unwrap();
        let [Stmt::If(branch)] = first.body.as_slice() else {
            panic!()
        };
        let Expr::Await(after) = after.value.as_ref() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, true).unwrap();
        assert_eq!(plan.remaining[&NativeRole::TryBody(statement.range)], 2);
        assert_eq!(plan.remaining[&NativeRole::TryExcept(statement.range)], 3);
        assert_eq!(plan.remaining[&NativeRole::TryEnd(statement.range)], 4);
        assert_eq!(plan.remaining[&NativeRole::TryCleanup(statement.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::HandlerNext(first.range)], 9);
        // Native allocates named cleanup before body, regardless of Serc's
        // physical label allocation order. The handler If and Await follow.
        assert_eq!(
            plan.remaining[&NativeRole::HandlerCleanupEnd(first.range)],
            10
        );
        assert_eq!(
            plan.remaining[&NativeRole::HandlerCleanupBody(first.range)],
            11
        );
        assert_eq!(plan.remaining[&NativeRole::IfEnd(branch.range)], 12);
        assert_eq!(plan.remaining[&NativeRole::HandlerNext(second.range)], 16);
        assert_eq!(
            plan.remaining[&NativeRole::HandlerCleanupBody(second.range)],
            17
        );
        assert!(
            !plan
                .remaining
                .contains_key(&NativeRole::HandlerCleanupEnd(second.range))
        );
        assert_eq!(plan.remaining[&NativeRole::AwaitSend(after.range)], 18);
        assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 21);
    }

    #[test]
    fn original_handler_cleanup_separates_dead_branch_end_from_next_handler() {
        let parsed = parse_module("def f(flag):\n    if flag:\n        try:\n            pass\n        except Error:\n            if flag:\n                raise Error\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::If(outer)] = function.body.as_slice() else {
            panic!()
        };
        let [Stmt::Try(statement)] = outer.body.as_slice() else {
            panic!()
        };
        let handler = statement.handlers[0].as_except_handler().unwrap();
        let [Stmt::If(inner)] = handler.body.as_slice() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        // TryEnd and the enclosing IfEnd mark the same original boundary.
        // The later native allocation wins, while dead normal cleanup keeps
        // the handler's authored IfEnd distinct from HandlerNext.
        assert_eq!(plan.remaining[&NativeRole::IfEnd(outer.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::TryEnd(statement.range)], 5);
        assert_eq!(plan.remaining[&NativeRole::IfEnd(inner.range)], 9);
        assert_eq!(plan.remaining[&NativeRole::HandlerNext(handler.range)], 7);
        assert_eq!(plan.remaining[&NativeRole::TryCleanup(statement.range)], 6);
    }

    #[test]
    fn ordinary_handler_loop_exits_need_no_new_native_labels() {
        for transfer in ["break", "continue"] {
            for binding in ["Error", "Error as error"] {
                let source = format!(
                    "def f(items):\n    for item in items:\n        try:\n            action(item)\n        except {binding}:\n            {transfer}\n    done()\n"
                );
                let parsed = parse_module(&source).unwrap();
                let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                    panic!()
                };
                assert!(
                    NativeLabelPlan::function(&function.body, false).is_some(),
                    "{source}"
                );
            }
        }
        let source = "def f(items, manager):\n    for item in items:\n        try:\n            action(item)\n        except Error:\n            with manager:\n                continue\n";
        let parsed = parse_module(source).unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        assert!(NativeLabelPlan::function(&function.body, false).is_none());
    }

    #[test]
    fn unsupported_handlers_reject_the_entire_native_label_plan() {
        for source in [
            "def f():\n    try:\n        action()\n    except:\n        pass\n",
            "def f():\n    try:\n        action()\n    except Error:\n        pass\n    else:\n        done()\n",
            "def f():\n    try:\n        action()\n    except Error:\n        pass\n    finally:\n        done()\n",
            "def f():\n    try:\n        action()\n    except* Error:\n        pass\n",
            "def f():\n    try:\n        action()\n    except Error:\n        if flag:\n            return value\n",
            "def f():\n    try:\n        action()\n    except Error:\n        try:\n            return value\n        except Other:\n            pass\n",
            "def f():\n    try:\n        action()\n    except (Error if flag else Other):\n        pass\n",
        ] {
            let parsed = parse_module(source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, false).is_none(),
                "{source}"
            );
        }
        // A child function owns a separate counter and handler stack.
        let parsed = parse_module("def f():\n    try:\n        action()\n    except Error:\n        def child():\n            return value\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        assert!(NativeLabelPlan::function(&function.body, false).is_some());
    }
}

#[cfg(test)]
mod comprehension_family_tests {
    use super::{NativeLabelPlan, NativeRole};
    use ruff_python_ast::{Expr, Stmt};
    use ruff_python_parser::parse_module;
    use ruff_text_size::Ranged;

    #[test]
    fn comprehension_clauses_interleave_filter_and_result_labels() {
        let parsed = parse_module("def f():\n    values = [(a if p else b) or c for x in xs if p or q for y, z in ys if q and r]\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::ListComp(comprehension) = assignment.value.as_ref() else {
            panic!()
        };
        let [outer, inner] = comprehension.generators.as_slice() else {
            panic!()
        };
        let Expr::BoolOp(result) = comprehension.elt.as_ref() else {
            panic!()
        };
        let Expr::If(conditional) = &result.values[0] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionCleanup(outer.iter.range())],
            2
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionEnd(outer.iter.range())],
            3
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionStart(outer.iter.range())],
            4
        );
        assert_eq!(
            plan.remaining[&NativeRole::ConditionBoolEnd(outer.ifs[0].range())],
            7
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionStart(inner.iter.range())],
            8
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionIfCleanup(inner.iter.range())],
            9
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionAnchor(inner.iter.range())],
            10
        );
        assert!(
            !plan
                .remaining
                .contains_key(&NativeRole::ConditionBoolEnd(inner.ifs[0].range()))
        );
        assert_eq!(plan.remaining[&NativeRole::ValueBoolEnd(result.range)], 11);
        assert_eq!(
            plan.remaining[&NativeRole::ValueIfEnd(conditional.range)],
            12
        );
        assert_eq!(
            plan.remaining[&NativeRole::ValueIfElse(conditional.range)],
            13
        );
        assert_eq!(plan.remaining.len(), 13);
    }

    #[test]
    fn inner_clause_reserves_labels_before_its_iterable_and_dictionary_result() {
        let parsed = parse_module("def f():\n    values = {(a if p else b): (c or d) for x in xs for y in (ys if flag else zs)}\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::DictComp(comprehension) = assignment.value.as_ref() else {
            panic!()
        };
        let inner = &comprehension.generators[1];
        let Expr::If(iterable) = &inner.iter else {
            panic!()
        };
        let Some(Expr::If(key)) = comprehension.key.as_deref() else {
            panic!()
        };
        let Expr::BoolOp(value) = comprehension.value.as_ref() else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionStart(inner.iter.range())],
            7
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionIfCleanup(inner.iter.range())],
            8
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionAnchor(inner.iter.range())],
            9
        );
        assert_eq!(plan.remaining[&NativeRole::ValueIfEnd(iterable.range)], 10);
        assert_eq!(plan.remaining[&NativeRole::ValueIfElse(iterable.range)], 11);
        assert_eq!(plan.remaining[&NativeRole::ValueIfEnd(key.range)], 12);
        assert_eq!(plan.remaining[&NativeRole::ValueIfElse(key.range)], 13);
        assert_eq!(plan.remaining[&NativeRole::ValueBoolEnd(value.range)], 14);
    }

    #[test]
    fn trailing_result_joins_alias_before_the_append_boundary() {
        let parsed =
            parse_module("def f():\n    values = [a or (b if p else c) for x in xs]\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::ListComp(comprehension) = assignment.value.as_ref() else {
            panic!()
        };
        let Expr::BoolOp(result) = comprehension.elt.as_ref() else {
            panic!()
        };
        let Expr::If(conditional) = &result.values[1] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        // The later IfEnd wins the shared original offset; append keeps the
        // clause's IfCleanup separate even if an optimizer later folds an arm.
        assert_eq!(plan.remaining[&NativeRole::ValueBoolEnd(result.range)], 8);
        assert_eq!(
            plan.remaining[&NativeRole::ValueIfEnd(conditional.range)],
            8
        );
        assert_eq!(
            plan.remaining
                [&NativeRole::ComprehensionIfCleanup(comprehension.generators[0].iter.range())],
            5
        );
    }
}

#[cfg(test)]
mod expression_family_tests {
    use super::{NativeLabelPlan, NativeRole};
    use ruff_python_ast::{Expr, Stmt};
    use ruff_python_parser::parse_module;
    use ruff_text_size::Ranged;

    #[test]
    fn value_chain_allocates_cleanup_after_left_and_end_after_comparators() {
        let parsed =
            parse_module("def f():\n    value = (a if p else b) < mid < (c if q else d)\n")
                .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::Compare(comparison) = assignment.value.as_ref() else {
            panic!()
        };
        let Expr::If(left) = comparison.left.as_ref() else {
            panic!()
        };
        let Expr::If(right) = &comparison.comparators[1] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        for (role, ordinal) in [
            (NativeRole::ValueIfEnd(left.range), 2),
            (NativeRole::ValueIfElse(left.range), 3),
            (NativeRole::ValueCompareCleanup(comparison.range), 4),
            (NativeRole::ValueIfEnd(right.range), 5),
            (NativeRole::ValueIfElse(right.range), 6),
            (NativeRole::ValueCompareEnd(comparison.range), 7),
        ] {
            assert_eq!(plan.remaining[&role], ordinal);
        }
        assert_eq!(plan.remaining.len(), 7);
    }

    #[test]
    fn condition_chain_allocates_cleanup_before_left_and_end_after_comparators() {
        let parsed = parse_module(
            "def f():\n    if (a if p else b) < mid < (c if q else d):\n        pass\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::If(statement)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::Compare(comparison) = statement.test.as_ref() else {
            panic!()
        };
        let Expr::If(left) = comparison.left.as_ref() else {
            panic!()
        };
        let Expr::If(right) = &comparison.comparators[1] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        for (role, ordinal) in [
            (NativeRole::IfEnd(statement.range), 2),
            (NativeRole::ConditionCompareCleanup(comparison.range), 3),
            (NativeRole::ValueIfEnd(left.range), 4),
            (NativeRole::ValueIfElse(left.range), 5),
            (NativeRole::ValueIfEnd(right.range), 6),
            (NativeRole::ValueIfElse(right.range), 7),
            (NativeRole::ConditionCompareEnd(comparison.range), 8),
        ] {
            assert_eq!(plan.remaining[&role], ordinal);
        }
        assert_eq!(plan.remaining.len(), 8);
    }

    #[test]
    fn chain_end_aliases_enclosing_boolean_join_in_both_contexts() {
        let parsed = parse_module(
            "def f():\n    value = p or a < b < c\n    if p or a < b < c:\n        pass\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment), Stmt::If(statement)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::BoolOp(value) = assignment.value.as_ref() else {
            panic!()
        };
        let Expr::Compare(value_chain) = &value.values[1] else {
            panic!()
        };
        let Expr::BoolOp(condition) = statement.test.as_ref() else {
            panic!()
        };
        let Expr::Compare(condition_chain) = &condition.values[1] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        assert_eq!(plan.remaining[&NativeRole::ValueBoolEnd(value.range)], 4);
        assert_eq!(
            plan.remaining[&NativeRole::ValueCompareEnd(value_chain.range)],
            4
        );
        assert_eq!(
            plan.remaining[&NativeRole::ValueCompareCleanup(value_chain.range)],
            3
        );
        assert_eq!(
            plan.remaining[&NativeRole::ConditionBoolEnd(condition.range)],
            8
        );
        assert_eq!(
            plan.remaining[&NativeRole::ConditionCompareEnd(condition_chain.range)],
            8
        );
        assert_eq!(
            plan.remaining[&NativeRole::ConditionCompareCleanup(condition_chain.range)],
            7
        );
        assert_eq!(plan.remaining[&NativeRole::IfEnd(statement.range)], 5);
    }

    #[test]
    fn nested_comprehension_schedules_interleave_filters_and_keep_restore_boundaries() {
        let parsed = parse_module(
            "def f():\n    values = [[z for z in zs if a < b < c] for x in xs if p or q]\n",
        )
        .unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::ListComp(outer) = assignment.value.as_ref() else {
            panic!()
        };
        let Expr::ListComp(inner) = outer.elt.as_ref() else {
            panic!()
        };
        let first = &outer.generators[0];
        let second = &inner.generators[0];
        let Expr::Compare(comparison) = &second.ifs[0] else {
            panic!()
        };
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        for (role, ordinal) in [
            (NativeRole::ComprehensionCleanup(first.iter.range()), 2),
            (NativeRole::ComprehensionEnd(first.iter.range()), 3),
            (NativeRole::ComprehensionStart(first.iter.range()), 4),
            (NativeRole::ComprehensionIfCleanup(first.iter.range()), 5),
            (NativeRole::ComprehensionAnchor(first.iter.range()), 6),
            (NativeRole::ConditionBoolEnd(first.ifs[0].range()), 7),
            (NativeRole::ComprehensionCleanup(second.iter.range()), 8),
            (NativeRole::ComprehensionEnd(second.iter.range()), 9),
            (NativeRole::ComprehensionStart(second.iter.range()), 10),
            (NativeRole::ComprehensionIfCleanup(second.iter.range()), 11),
            (NativeRole::ComprehensionAnchor(second.iter.range()), 12),
            (NativeRole::ConditionCompareCleanup(comparison.range), 13),
            (NativeRole::ConditionCompareEnd(comparison.range), 14),
        ] {
            assert_eq!(plan.remaining[&role], ordinal);
        }
        assert_eq!(plan.remaining.len(), 14);
    }

    #[test]
    fn nested_outer_iterable_completes_before_enclosing_save_state() {
        let parsed = parse_module("def f():\n    values = [x for x in [y for y in ys]]\n").unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!()
        };
        let [Stmt::Assign(assignment)] = function.body.as_slice() else {
            panic!()
        };
        let Expr::ListComp(outer) = assignment.value.as_ref() else {
            panic!()
        };
        let Expr::ListComp(inner) = &outer.generators[0].iter else {
            panic!()
        };
        let outer_range = outer.generators[0].iter.range();
        let inner_range = inner.generators[0].iter.range();
        let plan = NativeLabelPlan::function(&function.body, false).unwrap();
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionCleanup(inner_range)],
            2
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionEnd(inner_range)],
            3
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionAnchor(inner_range)],
            6
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionCleanup(outer_range)],
            7
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionEnd(outer_range)],
            8
        );
        assert_eq!(
            plan.remaining[&NativeRole::ComprehensionAnchor(outer_range)],
            11
        );
        assert_eq!(plan.remaining.len(), 11);
    }

    #[test]
    fn nested_comprehensions_restore_enclosing_suspension_restrictions() {
        for expression in [
            "[[y for y in ys] or await action() for x in xs]",
            "[([y for y in ys], await action()) for x in xs]",
            "[[y for y in await items()] for x in xs]",
            "[[y for y in ys for z in [item]] for x in xs]",
            "[[y async for y in ys] for x in xs]",
        ] {
            let source = format!("async def f():\n    values = {expression}\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            assert!(
                NativeLabelPlan::function(&function.body, true).is_none(),
                "{source}"
            );
        }
        for expression in [
            "[[y for y in ys] for x in await items()]",
            "[x for x in [y for y in await items()]]",
        ] {
            let source = format!("async def f():\n    values = {expression}\n    await done()\n");
            let parsed = parse_module(&source).unwrap();
            let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
                panic!()
            };
            let plan = NativeLabelPlan::function(&function.body, true).unwrap();
            assert_eq!(plan.remaining[&NativeRole::StopIterationHandler], 18);
            assert_eq!(plan.remaining.len(), 18);
        }
    }
}
