//! Conservative proof that all original Python 3.12 jump seeds fit in one byte.
//!
//! Count source labels S, true jumps J, and scope exits E before dead-code removal.
//! There are at most U = J + E + 1 original unlabeled blocks and B = S + U blocks.
//! Empty-block repair and cold placement label at most U original blocks. Cold
//! placement adds at most C = min(J, floor(B / 2)) bridges: each cold-to-warm
//! adjacency needs a distinct incoming warm jump because its cold predecessor
//! cannot make the successor warm. Exit duplication adds at most J + C labels.
//! Normalization adds unlabeled bridges only, so S + U + J + C bounds every label.
//!
//! This certificate controls initial `EXTENDED_ARG` widths, not final jump distances.
//! Unsupported syntax declines the complete code object. In particular, finally
//! copies and pattern failure stacks require additional source-generation proofs.

use ruff_python_ast::{
    Comprehension, Expr, FStringPart, InterpolatedStringElement, Parameters, Stmt, UnaryOp,
};

/// Proves that native source generation and every CFG allocation stay below 256.
pub(super) fn certifies(body: &[Stmt]) -> bool {
    function_counts(body).is_some_and(|counts| counts.max_label() < 256)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Counts {
    labels: u32,
    jumps: u32,
    exits: u32,
}

impl Counts {
    fn add(&mut self, labels: u32, jumps: u32, exits: u32) {
        self.labels = self.labels.saturating_add(labels);
        self.jumps = self.jumps.saturating_add(jumps);
        self.exits = self.exits.saturating_add(exits);
    }

    fn max_label(self) -> u32 {
        let unlabeled = self.jumps.saturating_add(self.exits).saturating_add(1);
        let blocks = self.labels.saturating_add(unlabeled);
        let cold_bridges = self.jumps.min(blocks / 2);
        blocks
            .saturating_add(self.jumps)
            .saturating_add(cold_bridges)
    }
}

#[derive(Default)]
struct Counter {
    counts: Counts,
    async_managers: u32,
    has_await: bool,
}

fn function_counts(body: &[Stmt]) -> Option<Counts> {
    let mut counter = Counter::default();
    // Function start, optional StopIteration handler, its RETURN/RERAISE, and
    // the implicit footer. This also overcounts modules: their prologue adds no
    // labels or jumps, and their footer adds only one scope exit.
    counter.counts.add(2, 0, 3);
    counter.suite(body)?;
    Some(counter.counts)
}

fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

impl Counter {
    fn suite(&mut self, statements: &[Stmt]) -> Option<()> {
        // Native source generation visits discarded suites and post-return tails.
        for statement in statements {
            if self.counts.max_label() >= 256 {
                return None;
            }
            self.statement(statement)?;
        }
        Some(())
    }

    fn statement(&mut self, statement: &Stmt) -> Option<()> {
        match statement {
            Stmt::Assign(assignment) => {
                self.expression(&assignment.value)?;
                for target in &assignment.targets {
                    self.expression(target)?;
                }
            }
            Stmt::AnnAssign(assignment) => {
                self.expression(&assignment.target)?;
                self.optional(assignment.value.as_deref())?;
                // Local annotations may not execute, but counting them is safe.
                self.expression(&assignment.annotation)?;
            }
            Stmt::AugAssign(assignment) => {
                self.expression(&assignment.target)?;
                self.expression(&assignment.value)?;
            }
            Stmt::Delete(deletion) => {
                for target in &deletion.targets {
                    self.expression(target)?;
                }
            }
            Stmt::Expr(expression) => self.expression(&expression.value)?,
            Stmt::Return(statement) => {
                self.unwind();
                self.counts.add(0, 0, 1);
                self.optional(statement.value.as_deref())?;
            }
            Stmt::Break(_) | Stmt::Continue(_) => {
                // Some loop exits stop before outer managers; charging all
                // active managers is conservative and needs no frame-stack copy.
                self.unwind();
                self.counts.add(0, 1, 0);
            }
            Stmt::Raise(statement) => {
                self.counts.add(0, 0, 1);
                self.optional(statement.exc.as_deref())?;
                self.optional(statement.cause.as_deref())?;
            }
            Stmt::Assert(statement) => {
                self.counts.add(1, 0, 1);
                self.condition(&statement.test)?;
                self.optional(statement.msg.as_deref())?;
            }
            Stmt::If(statement) => {
                let clauses = &statement.elif_else_clauses;
                let has_else = u32::from(!clauses.is_empty());
                self.counts.add(1 + has_else, has_else, 0);
                self.condition(&statement.test)?;
                self.suite(&statement.body)?;
                // Ruff flattens elif; native recursively emits another If with
                // its own end and optional else label for every tested clause.
                for (index, clause) in clauses.iter().enumerate() {
                    if let Some(test) = &clause.test {
                        let has_else = u32::from(index + 1 < clauses.len());
                        self.counts.add(1 + has_else, has_else, 0);
                        self.condition(test)?;
                    }
                    self.suite(&clause.body)?;
                }
            }
            Stmt::While(statement) => {
                self.counts.add(4, 0, 0);
                self.condition(&statement.test)?;
                self.suite(&statement.body)?;
                self.condition(&statement.test)?;
                self.suite(&statement.orelse)?;
            }
            Stmt::For(statement) => {
                let (labels, jumps) = if statement.is_async { (6, 3) } else { (4, 2) };
                self.counts.add(labels, jumps, 0);
                self.expression(&statement.iter)?;
                self.expression(&statement.target)?;
                self.suite(&statement.body)?;
                self.suite(&statement.orelse)?;
            }
            Stmt::With(statement) => {
                let (labels, jumps) = if statement.is_async { (15, 9) } else { (6, 3) };
                for item in &statement.items {
                    self.counts.add(labels, jumps, 2);
                    self.expression(&item.context_expr)?;
                    self.optional(item.optional_vars.as_deref())?;
                }
                let previous = self.async_managers;
                if statement.is_async {
                    self.async_managers = previous.saturating_add(count(statement.items.len()));
                }
                self.suite(&statement.body)?;
                self.async_managers = previous;
            }
            Stmt::Try(statement) => {
                if !statement.finalbody.is_empty() {
                    return None;
                }
                if statement.is_star {
                    self.counts.add(7, 4, 2);
                } else {
                    self.counts.add(4, 1, 2);
                }
                self.suite(&statement.body)?;
                self.suite(&statement.orelse)?;
                for handler in &statement.handlers {
                    let handler = handler.as_except_handler()?;
                    if statement.is_star {
                        self.counts.add(5, 4, 0);
                    } else {
                        self.counts.add(
                            if handler.name.is_some() { 3 } else { 2 },
                            1 + u32::from(handler.type_.is_some()),
                            u32::from(handler.name.is_some()),
                        );
                    }
                    self.optional(handler.type_.as_deref())?;
                    self.suite(&handler.body)?;
                }
            }
            Stmt::FunctionDef(definition) => {
                if definition.type_params.is_some() {
                    return None;
                }
                for decorator in &definition.decorator_list {
                    self.expression(&decorator.expression)?;
                }
                self.parameters(&definition.parameters)?;
                self.optional(definition.returns.as_deref())?;
                // The child body has an independent native label counter.
            }
            Stmt::ClassDef(definition) => {
                if definition.type_params.is_some() {
                    return None;
                }
                for decorator in &definition.decorator_list {
                    self.expression(&decorator.expression)?;
                }
                if let Some(arguments) = &definition.arguments {
                    for argument in &arguments.args {
                        self.expression(argument)?;
                    }
                    for keyword in &arguments.keywords {
                        self.expression(&keyword.value)?;
                    }
                }
            }
            Stmt::Pass(_)
            | Stmt::Import(_)
            | Stmt::ImportFrom(_)
            | Stmt::Global(_)
            | Stmt::Nonlocal(_) => {}
            _ => return None,
        }
        Some(())
    }

    fn unwind(&mut self) {
        self.counts.add(
            self.async_managers.saturating_mul(3),
            self.async_managers.saturating_mul(2),
            0,
        );
    }

    fn parameters(&mut self, parameters: &Parameters) -> Option<()> {
        for parameter in parameters.iter_non_variadic_params() {
            self.optional(parameter.default.as_deref())?;
            self.optional(parameter.parameter.annotation.as_deref())?;
        }
        for parameter in [parameters.vararg.as_deref(), parameters.kwarg.as_deref()]
            .into_iter()
            .flatten()
        {
            self.optional(parameter.annotation.as_deref())?;
        }
        Some(())
    }

    fn optional(&mut self, expression: Option<&Expr>) -> Option<()> {
        if let Some(expression) = expression {
            self.expression(expression)?;
        }
        Some(())
    }

    fn condition(&mut self, expression: &Expr) -> Option<()> {
        match expression {
            Expr::UnaryOp(unary) if unary.op == UnaryOp::Not => self.condition(&unary.operand)?,
            Expr::BoolOp(boolean) => {
                // At most one private short-circuit label in either polarity.
                self.counts.add(1, 0, 0);
                for value in &boolean.values {
                    self.condition(value)?;
                }
            }
            Expr::If(conditional) => {
                self.counts.add(2, 1, 0);
                self.condition(&conditional.test)?;
                self.condition(&conditional.body)?;
                self.condition(&conditional.orelse)?;
            }
            Expr::Compare(comparison) if comparison.ops.len() > 1 => {
                self.counts
                    .add(2, count(comparison.ops.len()).saturating_add(2), 0);
                self.expression(&comparison.left)?;
                for comparator in &comparison.comparators {
                    self.expression(comparator)?;
                }
            }
            _ => {
                self.expression(expression)?;
                self.counts.add(0, 1, 0);
            }
        }
        Some(())
    }

    fn comprehension(
        &mut self,
        generators: &[Comprehension],
        element: &Expr,
        key: Option<&Expr>,
    ) -> Option<()> {
        generators.first()?;
        let outer_await = std::mem::take(&mut self.has_await);
        // Count locals cleanup even when native needs no locals preservation.
        self.counts.add(2, 1, 1);
        let mut is_async = false;
        for generator in generators {
            let (labels, jumps) = if generator.is_async { (6, 3) } else { (3, 2) };
            self.counts.add(labels, jumps, 0);
            is_async |= generator.is_async;
            self.expression(&generator.iter)?;
            self.expression(&generator.target)?;
            for condition in &generator.ifs {
                self.condition(condition)?;
            }
        }
        self.optional(key)?;
        self.expression(element)?;
        if is_async || self.has_await {
            // Inlining usually avoids this; bound a possible coroutine caller
            // await as well, propagating it to any enclosing comprehension.
            self.counts.add(3, 2, 0);
            self.has_await = true;
        }
        self.has_await |= outer_await;
        Some(())
    }

    fn interpolated_elements(
        &mut self,
        elements: &ruff_python_ast::InterpolatedStringElements,
    ) -> Option<()> {
        for element in elements {
            if let InterpolatedStringElement::Interpolation(interpolation) = element {
                self.expression(&interpolation.expression)?;
                if let Some(spec) = &interpolation.format_spec {
                    self.interpolated_elements(&spec.elements)?;
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
            Expr::Await(awaited) => {
                self.expression(&awaited.value)?;
                self.counts.add(3, 2, 0);
                self.has_await = true;
            }
            Expr::YieldFrom(yielded) => {
                self.expression(&yielded.value)?;
                self.counts.add(3, 2, 0);
            }
            Expr::Yield(yielded) => self.optional(yielded.value.as_deref())?,
            Expr::If(conditional) => {
                self.counts.add(2, 1, 0);
                self.condition(&conditional.test)?;
                self.expression(&conditional.body)?;
                self.expression(&conditional.orelse)?;
            }
            Expr::BoolOp(boolean) => {
                self.counts
                    .add(1, count(boolean.values.len()).saturating_sub(1), 0);
                for value in &boolean.values {
                    self.expression(value)?;
                }
            }
            Expr::Compare(comparison) => {
                if comparison.ops.len() > 1 {
                    self.counts.add(2, count(comparison.ops.len()), 0);
                }
                self.expression(&comparison.left)?;
                for comparator in &comparison.comparators {
                    self.expression(comparator)?;
                }
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.parameters(parameters)?;
                }
            }
            Expr::Generator(generator) => self.expression(&generator.generators.first()?.iter)?,
            Expr::ListComp(comprehension) => {
                self.comprehension(&comprehension.generators, &comprehension.elt, None)?;
            }
            Expr::SetComp(comprehension) => {
                self.comprehension(&comprehension.generators, &comprehension.elt, None)?;
            }
            Expr::DictComp(comprehension) => {
                self.comprehension(
                    &comprehension.generators,
                    &comprehension.value,
                    Some(comprehension.key.as_deref()?),
                )?;
            }
            Expr::Attribute(attribute) => self.expression(&attribute.value)?,
            Expr::Subscript(subscript) => {
                self.expression(&subscript.value)?;
                self.expression(&subscript.slice)?;
            }
            Expr::Slice(slice) => {
                self.optional(slice.lower.as_deref())?;
                self.optional(slice.upper.as_deref())?;
                self.optional(slice.step.as_deref())?;
            }
            Expr::Named(named) => {
                self.expression(&named.value)?;
                self.expression(&named.target)?;
            }
            Expr::UnaryOp(unary) => self.expression(&unary.operand)?,
            Expr::BinOp(binary) => {
                self.expression(&binary.left)?;
                self.expression(&binary.right)?;
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
            Expr::Dict(dictionary) => {
                for item in &dictionary.items {
                    self.optional(item.key.as_ref())?;
                    self.expression(&item.value)?;
                }
            }
            Expr::FString(fstring) => {
                for part in &fstring.value {
                    if let FStringPart::FString(part) = part {
                        self.interpolated_elements(&part.elements)?;
                    }
                }
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Counts, certifies, function_counts};
    use ruff_python_ast::Stmt;
    use ruff_python_parser::parse_module;

    fn counts(source: &str) -> Option<Counts> {
        let parsed = parse_module(source).unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!("expected one function");
        };
        function_counts(&function.body)
    }

    #[test]
    fn strict_bound_and_overflow_decline_extended_native_seeds() {
        let below = Counts {
            labels: 1,
            jumps: 99,
            exits: 3,
        };
        let boundary = Counts { labels: 2, ..below };
        assert_eq!(below.max_label(), 255);
        assert_eq!(boundary.max_label(), 256);
        let mut overflow = Counts {
            labels: u32::MAX,
            jumps: u32::MAX,
            exits: u32::MAX,
        };
        overflow.add(1, 1, 1);
        assert_eq!(overflow.max_label(), u32::MAX);
    }

    #[test]
    fn warm_jump_bound_certifies_sparse_cold_adjacencies() {
        let source = format!("async def f():\n{}", "    await work()\n".repeat(26));
        let budget = counts(&source).unwrap();
        assert_eq!(
            budget,
            Counts {
                labels: 80,
                jumps: 52,
                exits: 3,
            },
        );
        // The physical-adjacency bound alone gives 256, but only 52 warm
        // jumps can reach distinct successors of cold blocks, giving 240.
        assert_eq!(budget.max_label(), 240);
        let parsed = parse_module(&source).unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!("expected one function");
        };
        assert!(certifies(&function.body));

        let source = format!("async def f():\n{}", "    await work()\n".repeat(28));
        assert_eq!(counts(&source).unwrap().max_label(), 258);
        let parsed = parse_module(&source).unwrap();
        let [Stmt::FunctionDef(function)] = parsed.suite().as_slice() else {
            panic!("expected one function");
        };
        assert!(!certifies(&function.body));
    }

    #[test]
    fn module_body_and_discarded_labels_keep_the_strict_boundary() {
        let source = format!("{}raise Error\n", "if flag:\n    pass\n".repeat(62));
        let parsed = parse_module(&source).unwrap();
        assert_eq!(function_counts(parsed.suite()).unwrap().max_label(), 255);
        assert!(certifies(parsed.suite()));

        let source = format!("{source}raise Other\n");
        let parsed = parse_module(&source).unwrap();
        assert_eq!(function_counts(parsed.suite()).unwrap().max_label(), 256);
        assert!(!certifies(parsed.suite()));

        let source = format!("if False:\n{}", "    if flag:\n        pass\n".repeat(300));
        let parsed = parse_module(&source).unwrap();
        assert!(!certifies(parsed.suite()));
    }

    #[test]
    fn dead_tails_and_flattened_elif_keep_native_allocation_costs() {
        assert_eq!(
            counts(
                "def f(flag, other):\n    return\n    if flag:\n        pass\n    elif other:\n        pass\n    else:\n        pass\n"
            ),
            Some(Counts {
                labels: 6,
                jumps: 4,
                exits: 4
            }),
        );
        assert_eq!(
            counts("async def f():\n    while await test():\n        pass\n"),
            Some(Counts {
                labels: 12,
                jumps: 6,
                exits: 3
            }),
        );
    }

    #[test]
    fn async_manager_unwind_costs_include_each_active_manager() {
        assert_eq!(
            counts(
                "async def f():\n    async with first(), second():\n        return result\n    return other\n"
            ),
            Some(Counts {
                labels: 38,
                jumps: 22,
                exits: 9
            }),
        );
        // The outer manager is not actually unwound by this break; charging it
        // too is the deliberate upper bound, and its depth is restored after it.
        assert_eq!(
            counts(
                "async def f():\n    async with manager():\n        for item in items:\n            break\n"
            ),
            Some(Counts {
                labels: 24,
                jumps: 14,
                exits: 5
            }),
        );
    }

    #[test]
    fn handlers_include_implicit_reraises_and_star_helpers() {
        assert_eq!(
            counts(
                "def f():\n    try:\n        work()\n    except Error as error:\n        pass\n    except:\n        pass\n"
            ),
            Some(Counts {
                labels: 11,
                jumps: 4,
                exits: 6
            }),
        );
        assert_eq!(
            counts("def f():\n    try:\n        work()\n    except* Error:\n        pass\n"),
            Some(Counts {
                labels: 14,
                jumps: 8,
                exits: 5
            }),
        );
    }

    #[test]
    fn comprehensions_include_cleanup_and_possible_caller_await() {
        assert_eq!(
            counts(
                "async def f(items):\n    result = [item for item in items if await keep(item)]\n"
            ),
            Some(Counts {
                labels: 13,
                jumps: 8,
                exits: 4
            }),
        );
        // Nested asynchronous comprehensions also propagate suspension to the
        // outer conservative caller budget, without sharing native child labels.
        assert_eq!(
            counts(
                "async def f(groups):\n    result = [[item async for item in group] for group in groups]\n"
            ),
            Some(Counts {
                labels: 21,
                jumps: 11,
                exits: 5
            }),
        );
    }

    #[test]
    fn child_bodies_use_separate_counters_but_defaults_use_the_parent() {
        assert_eq!(
            counts(
                "def f(a, b):\n    def child(value=a or b):\n        try:\n            work()\n        finally:\n            finish()\n"
            ),
            Some(Counts {
                labels: 3,
                jumps: 1,
                exits: 3
            }),
        );
        assert_eq!(
            counts("async def f():\n    result = (await work(item) for item in await source())\n"),
            Some(Counts {
                labels: 5,
                jumps: 2,
                exits: 3
            }),
        );
    }

    #[test]
    fn unsupported_allocation_routes_decline_the_whole_function() {
        for source in [
            "def f():\n    try:\n        work()\n    finally:\n        finish()\n",
            "def f(value):\n    match value:\n        case _: pass\n",
            "def f():\n    def generic[T]():\n        pass\n",
            "def f():\n    type Alias = int\n",
        ] {
            assert_eq!(counts(source), None, "{source}");
        }
    }

    #[test]
    fn large_code_can_have_small_seeds_but_many_dead_labels_cannot() {
        let mut source = String::from("async def f(flag):\n");
        source.push_str(&"    value = 0\n".repeat(300));
        source.push_str("    values = [item for item in items]\n    await work()\n");
        let parsed = parse_module(&source).unwrap();
        let Stmt::FunctionDef(function) = &parsed.suite()[0] else {
            panic!()
        };
        assert!(certifies(&function.body));
        source.push_str("    return\n");
        source.push_str(&"    if flag:\n        pass\n".repeat(300));
        let parsed = parse_module(&source).unwrap();
        let Stmt::FunctionDef(function) = &parsed.suite()[0] else {
            panic!()
        };
        assert!(!certifies(&function.body));
    }
}
