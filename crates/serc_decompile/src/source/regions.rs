//! Structure a slice of the instruction graph. Each arm gets a fresh operand
//! stack; values that survive a join must be expressible as a Python expression.
use std::fmt::Write as _;

use super::{Body, Instruction, Object, Value};

pub(super) struct Fragment {
    lines: String,
    pub(super) stack: Vec<Value>,
}

impl Fragment {
    pub(super) fn operands(self) -> Result<Vec<Value>, String> {
        if !self.lines.is_empty() {
            return Err("statement in assignment target".into());
        }
        Ok(self.stack)
    }

    pub(super) fn expression(mut self) -> Result<String, String> {
        if self.lines.is_empty()
            && self.stack.len() == 1
            && let Some(Value::Expr(value)) = self.stack.pop()
        {
            Ok(value)
        } else {
            Err("region is not a single expression".into())
        }
    }
    pub(super) fn key_value(mut self) -> Result<(String, String), String> {
        let Some(Value::Expr(value)) = self.stack.pop() else {
            return Err("missing comprehension value".into());
        };
        Ok((self.expression()?, value))
    }

    pub(super) fn pending_return(mut self) -> Result<(String, bool), String> {
        match self.stack.pop() {
            None => Ok((self.lines, false)),
            Some(Value::Expr(value)) if self.stack.is_empty() => {
                writeln!(self.lines, "return {value}").unwrap();
                Ok((self.lines, true))
            }
            _ => Err("unsupported protected stack outcome".into()),
        }
    }
    pub(super) fn suite(self) -> Result<String, String> {
        if self.stack.is_empty() {
            Ok(self.lines)
        } else {
            Err("unresolved stack value at statement boundary".into())
        }
    }
}

impl Body<'_, '_, '_> {
    pub(super) fn fragment(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<Fragment, String> {
        self.seeded(ops, start, end, level, Vec::new())
    }

    pub(super) fn seeded(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
        stack: Vec<Value>,
    ) -> Result<Fragment, String> {
        let outer_stack = std::mem::replace(&mut self.stack, stack);
        let outer_lines = std::mem::take(&mut self.lines);
        let outer_loop = self.loop_edges;
        let result = self.region(ops, start, end, level + 1);
        self.loop_edges = outer_loop;
        let fragment = Fragment {
            lines: std::mem::replace(&mut self.lines, outer_lines),
            stack: std::mem::replace(&mut self.stack, outer_stack),
        };
        result.map(|()| fragment)
    }

    pub(super) fn block(&mut self, header: &str, suite: &str) -> Result<(), String> {
        self.line(header)?;
        if suite.is_empty() {
            self.line("    pass")?;
        }
        for line in suite.lines() {
            self.line(&format!("    {line}"))?;
        }
        Ok(())
    }

    pub(super) fn region(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<(), String> {
        stacker::maybe_grow(256 * 1024, 2 * 1024 * 1024, || {
            self.recover_region(ops, start, end, level)
        })
    }

    fn recover_region(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        level: usize,
    ) -> Result<(), String> {
        if level > 32 || end > ops.len() || start > end {
            return Err("unsupported region bounds or nesting".into());
        }
        let partial = self.work.partial
            && level == 0
            && start < end
            && self.stack.is_empty()
            && self.flow.unprotected_suite(start..end);
        let mut prefix = self.lines.len();
        let mut i = start;
        while i < end {
            if partial && self.stack.is_empty() {
                prefix = self.lines.len();
            }
            let op = ops[i];
            let next = match self.region_step(ops, i, end, level) {
                Ok(next) => next,
                Err(reason) if partial => {
                    self.lines.truncate(prefix);
                    self.stack.clear();
                    let next = self
                        .resume_after(ops, i)?
                        .filter(|&next| next <= end)
                        .unwrap_or(end);
                    self.gap(&reason, &ops[i..next], next == ops.len())?;
                    next
                }
                Err(reason) => return Err(format!("byte {} ({}): {reason}", op.offset, op.name)),
            };
            if next <= i || next > end {
                return Err(format!("branch escapes region at byte {}", op.offset));
            }
            if partial && self.stack.is_empty() && self.lines.len() > prefix {
                self.charge(self.lines.len() - prefix)?;
                if let Err(error) = ruff_python_parser::parse_module(&self.lines[prefix..]) {
                    self.lines.truncate(prefix);
                    self.gap(
                        &format!("invalid reconstructed statement: {error}"),
                        &ops[i..next],
                        false,
                    )?;
                }
            }
            i = next;
        }
        Ok(())
    }

    fn region_step(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let op = ops[i];
        match op.name {
            // CPython builds a constant set display by updating an empty
            // set from a frozenset constant. Its elements belong directly
            // in the display, including when that display is itself starred.
            "BUILD_SET"
                if op.arg == 0
                    && let [constant, update, ..] = &ops[i + 1..end]
                    && constant.name == "LOAD_CONST"
                    && (update.name, update.arg) == ("SET_UPDATE", 1)
                    && matches!(
                        self.m.objects[self.item(self.code.constants, constant.arg)?],
                        Object::FrozenSet(_)
                    ) =>
            {
                self.constant(constant.arg)?;
                Ok(i + 3)
            }
            "BUILD_TUPLE"
                if op.arg == 3
                    && ops[i + 1..end]
                        .first()
                        .is_some_and(super::type_params::is_alias_intrinsic) =>
            {
                self.make_alias()?;
                Ok(i + 2)
            }
            "CALL_INTRINSIC_1"
                if op.arg
                    == serc_bytecode::target::operands::UnaryIntrinsic::IMPORT_STAR.argument()
                    && i + 1 < end
                    && ops[i + 1].name == "POP_TOP" =>
            {
                self.import_star()?;
                Ok(i + 2)
            }
            "LOAD_LOCALS" if self.class && self.class_parameters(&ops[i..end])? => Ok(i + 3),
            "COPY"
                if self.class
                    && op.arg == 1
                    && let [store, ret, ..] = &ops[i + 1..end]
                    && store.name == "STORE_NAME"
                    && ret.name == "RETURN_VALUE"
                    && self.name(self.code.names, store.arg)? == "__classcell__"
                    && matches!(self.stack.last(), Some(Value::Expr(value)) if value == "__class__") =>
            {
                self.pop()?;
                Ok(i + 3)
            }
            "COPY"
                if op.arg == 1
                    && matches!(self.stack.last(), Some(Value::Expr(_)))
                    && let Ok(targets) = self.chained_targets(&ops[..end], i + 1) =>
            {
                let rhs = self.expr()?;
                self.line(&format!("{} = {rhs}", targets.target))?;
                self.stack.extend(targets.seed);
                Ok(targets.next)
            }
            "COPY"
                if op.arg == 1
                    && i + 1 < end
                    && matches!(
                        ops[i + 1].name,
                        "STORE_FAST" | "STORE_NAME" | "STORE_DEREF" | "STORE_GLOBAL"
                    ) =>
            {
                let rhs = self.expr()?;
                let store = ops[i + 1];
                let table = if matches!(store.name, "STORE_NAME" | "STORE_GLOBAL") {
                    self.code.names
                } else {
                    self.code.locals
                };
                let lhs = self.name(table, store.arg)?;
                self.push(format!("({lhs} := {rhs})"))?;
                Ok(i + 2)
            }
            "SET_ADD"
                if i + 1 < end && ops[i + 1].name == "POP_TOP" && !self.annotations.is_empty() =>
            {
                self.annotation_site()?;
                Ok(i + 2)
            }
            "SWAP" if super::comparisons::is_link(&ops[i..end]) => {
                self.comparison_chain(ops, i, end, level)
            }
            "SWAP"
                if op.arg == 2
                    && i + 1 < end
                    && ops[i + 1].name == "POP_TOP"
                    && matches!(
                        self.stack.as_slice(),
                        [.., Value::Import(_), Value::Member(_)]
                    ) =>
            {
                // A dotted import walks the package before binding its final member.
                self.pop()?;
                Ok(i + 2)
            }
            "SWAP"
                if matches!(op.arg, 2 | 3)
                    && matches!(
                        ops[i + 1..end].first().map(|op| op.name),
                        Some(
                            "STORE_NAME"
                                | "STORE_FAST"
                                | "STORE_FAST_STORE_FAST"
                                | "STORE_GLOBAL"
                                | "STORE_DEREF"
                                | "UNPACK_SEQUENCE"
                                | "UNPACK_EX"
                        )
                    ) =>
            {
                self.parallel_assignment(ops, i + 1, end, op.arg as usize, false)
            }
            "STORE_FAST" | "STORE_FAST_STORE_FAST"
                if super::operands::local_store_run(&ops[i..end], self.stack.len())
                    && self.stack.iter().all(|v| matches!(v, Value::Expr(_))) =>
            {
                self.parallel_assignment(ops, i, end, self.stack.len(), true)
            }
            "KW_NAMES" if i + 1 < end && ops[i + 1].name == "CALL" => {
                self.constant(op.arg)?;
                let keys = self.expr()?;
                self.keyword_call(ops[i + 1].arg, &keys)?;
                Ok(i + 2)
            }
            "UNPACK_SEQUENCE" | "UNPACK_EX" => {
                let rhs = self.expr()?;
                let binding = self.binding(ops, i, end)?;
                self.line(&format!("{} = {rhs}", binding.target))?;
                self.stack.extend(binding.seed);
                Ok(binding.next)
            }
            "RETURN_GENERATOR" if i + 1 < end && ops[i + 1].name == "POP_TOP" => Ok(i + 2),
            "GET_AWAITABLE" | "GET_YIELD_FROM_ITER" => self.delegated(ops, i, end),
            "GET_ITER" | "GET_AITER"
                if ops
                    .get(i + 1)
                    .is_some_and(|next| next.name == "CALL" && next.arg == 0) =>
            {
                Ok(i + 1)
            }
            "GET_ITER" if self.m.version == serc_bytecode::PythonVersion::Py315 && op.arg == 3 => {
                self.delegated(ops, i, end)
            }
            "GET_AITER" if i + 1 < end && ops[i + 1].name == "GET_ANEXT" => {
                self.async_loop(ops, i, end, level)
            }
            "GET_ITER" | "GET_AITER" if i + 1 < end && ops[i + 1].name == "LOAD_FAST_AND_CLEAR" => {
                self.comprehension(ops, i + 1, end, level)
            }
            "LOAD_FAST_AND_CLEAR" => self.comprehension(ops, i, end, level),
            "JUMP_BACKWARD" | "JUMP_FORWARD" if self.loop_transfer(ops, i).is_some() => {
                self.line(self.loop_transfer(ops, i).unwrap())?;
                Ok(i + 1)
            }
            "NOP" | "POP_TOP" | "POP_ITER"
                if self.stack.is_empty() && self.folded_break(ops, i, end) =>
            {
                self.line("break")?;
                Ok(self.flow.block(i).ops.end)
            }
            "POP_TOP" | "POP_ITER"
                if self.stack.is_empty()
                    && i + 1 < end
                    && (self.loop_transfer(ops, i + 1) == Some("break")
                        || (ops[i + 1].name == "POP_TOP"
                            && i + 2 < end
                            && self.loop_transfer(ops, i + 2) == Some("break"))) =>
            {
                Ok(i + 1)
            }
            "MATCH_SEQUENCE" | "MATCH_MAPPING" | "MATCH_CLASS" => {
                self.match_cases(ops, i, end, level)
            }
            "COPY"
                if i + 1 < end && matches!(ops[i + 1].name, "MATCH_SEQUENCE" | "MATCH_MAPPING") =>
            {
                self.match_cases(ops, i, end, level)
            }
            "COPY" if let Some(count) = super::operands::augmented_load(&ops[i..end]) => {
                self.augmented_store(ops, i + count, count, end, level)
            }
            "COPY" if super::value_patterns::header(&ops[..end], i, self.m.version).is_some() => {
                self.value_cases(ops, i, end, level)
            }
            "COPY" if op.arg == 1 => self.boolean(ops, i, end, level),
            "GET_ITER" if ops.get(i + 1).is_some_and(|next| next.name == "FOR_ITER") => {
                self.for_loop(ops, i + 1, end, level)
            }
            "POP_JUMP_IF_FALSE"
            | "POP_JUMP_IF_TRUE"
            | "POP_JUMP_IF_NONE"
            | "POP_JUMP_IF_NOT_NONE" => self.conditional(ops, i, end, level),
            "TO_BOOL"
                if ops.get(i + 1).is_some_and(|op| {
                    op.name.starts_with("POP_JUMP_IF_") || op.name == "UNARY_NOT"
                }) =>
            {
                Ok(i + 1)
            }
            "TO_BOOL" => {
                // CPython folds double negation to a truth conversion. A
                // call to bool would introduce a (possibly shadowed) name.
                let value = self.expr()?;
                self.push(format!("(not not {value})"))?;
                Ok(i + 1)
            }
            _ => self.step(op).map(|()| i + 1),
        }
    }

    fn boolean(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let branch = i + 1 + usize::from(ops.get(i + 1).is_some_and(|op| op.name == "TO_BOOL"));
        let operator = match ops.get(branch).map(|op| op.name) {
            Some("POP_JUMP_IF_FALSE") => "and",
            Some("POP_JUMP_IF_TRUE") => "or",
            _ => return Err("unsupported copied value".into()),
        };
        let discard =
            branch + 1 + usize::from(ops.get(branch + 1).is_some_and(|op| op.name == "NOT_TAKEN"));
        let target = self.flow.target(branch)?;
        if ops.get(discard).is_none_or(|op| op.name != "POP_TOP")
            || target > end
            || target <= discard + 1
        {
            return Err("unsupported short-circuit shape".into());
        }
        let lhs = self.expr()?;
        let rhs = self
            .fragment(ops, discard + 1, target, level)?
            .expression()?;
        self.push(format!("({lhs} {operator} {rhs})"))?;
        Ok(target)
    }
}
