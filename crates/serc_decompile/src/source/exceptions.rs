//! The exception table records protected intervals and unwind targets. CPython
//! duplicates cleanup on normal paths and places the exceptional copy out of line.
use super::conditionals::Predicate;
use super::{Body, Bytecode, Instruction};
use serc_bytecode::instructions::Handler;

pub(super) fn boundary(ops: &[Instruction], offset: usize) -> Result<usize, String> {
    ops.binary_search_by_key(&offset, |op| op.offset)
        .map_err(|_| "invalid exception boundary".into())
}

impl Body<'_, '_, '_> {
    pub(super) fn try_block(&mut self, suite: &str, inline: bool) -> Result<(), String> {
        // A one-line try suite can omit the tracing NOP before its first
        // protected instruction. The formatted candidate also tries a block.
        if inline && suite.lines().count() == 1
            && ruff_python_parser::parse_module(suite).is_ok_and(|parsed| {
                matches!(parsed.suite().as_slice(), [stmt] if !ruff_python_ast::helpers::is_compound_statement(stmt))
            })
        {
            self.line(&format!("try: {}", suite.trim_end()))?;
        } else {
            self.block("try:", suite)?;
        }
        Ok(())
    }

    pub(super) fn protected(
        &mut self,
        bc: &Bytecode,
        handlers: &[Handler],
        limit: usize,
    ) -> Result<(), String> {
        if handlers.len() == bc.handlers.len()
            && let Some((regions, end)) = super::protected_regions::guarded(bc, self)
        {
            self.handlers = regions;
            if self.depth > 0 && !self.class {
                self.index_terminal_returns(&bc.ops[..end], None);
            }
            return self.region(&bc.ops, 0, end, 0);
        }
        let entry = handlers[0];
        let ops = &bc.ops;
        let index = |offset| boundary(ops, offset);
        let start = index(entry.start)?;
        let end = index(entry.end)?;
        let handler = index(entry.target)?;
        let normal_end = handler.min(limit);
        if ops[start].name == "PUSH_EXC_INFO" && !self.flow.reachable(start) {
            return self.elided_tries(bc, start);
        }
        if handler < end || ops[handler].name != "PUSH_EXC_INFO" {
            return Err(format!(
                "unsupported exception region at byte {}",
                entry.start
            ));
        }
        if ops
            .get(handler + 1)
            .is_some_and(|op| op.name == "WITH_EXCEPT_START")
        {
            self.handlers = super::protected_regions::collect(bc, self);
            return self.with_statement(bc, handlers, 0, limit);
        }
        if entry.depth > 0 && !entry.lasti {
            // A protected condition inside a loop keeps the iterator on the
            // stack. Its exceptional cleanup is out of line.
            return self.region(ops, 0, normal_end, 0);
        }
        if entry.depth != 0 || entry.lasti {
            return Err(format!("unsupported handler depth at byte {}", entry.start));
        }
        if self.depth > 0
            && !self.class
            && (self.work.partial || ops.iter().any(|op| op.name == "WITH_EXCEPT_START"))
            && ops[..start].iter().any(|op| {
                op.name.starts_with("POP_JUMP")
                    && op.jump.is_some_and(|target| target > entry.start)
            })
            && let Some((regions, prefix_end)) =
                super::protected_regions::guarded_prefix(bc, self, limit)
        {
            if let Some(enter) = self.manager_after_prefix(ops, prefix_end) {
                self.handlers = regions;
                self.region(ops, 0, enter, 0)?;
                if !matches!(self.stack.as_slice(), [super::Value::Expr(_)]) {
                    return Err("unresolved context manager after guarded prefix".into());
                }
                let next = handlers.partition_point(|h| h.start < ops[prefix_end].offset);
                return self.with_statement(bc, &handlers[next..], enter, limit);
            }
            if self.work.partial {
                self.handlers = regions;
                self.region(ops, 0, prefix_end, 0)?;
                if self.stack.is_empty() {
                    self.partial_start = prefix_end;
                }
                return Err(format!(
                    "unplaced exception region at byte {}",
                    ops[prefix_end].offset
                ));
            }
        }
        self.region(ops, 0, start, 0)?;
        let return_exit = end + 1 == normal_end && ops[end].name == "RETURN_VALUE";
        let (suite, returns) = self
            .suite_fragment(ops, start..end, 0, Vec::new(), return_exit)?
            .pending_return()?;
        let try_source_start = self.lines.len();
        self.try_block(&suite, start > 0 && ops[start - 1].name != "NOP")?;
        let star = ops
            .get(handler + 1)
            .is_some_and(|op| op.name == "BUILD_LIST");
        if star && self.exception_group(ops, end..handler, returns)? {
            return Ok(());
        }
        if !star
            && returns
            // Inlined comprehension cleanups may follow the normal return
            // before the user's dispatcher. They own no normal continuation.
            && return_exit
            && ops.get(handler + 1).is_some_and(|op| {
                op.name == "POP_TOP" || self.flow
                    .block(handler + 1)
                    .ops
                    .clone()
                    .any(|i| ops[i].name == "CHECK_EXC_MATCH")
            })
        {
            return self.returning_handlers(ops, handler);
        }
        if !star
            && !returns
            && let Some(dispatch) = self.exception_dispatch(bc, handler, start, end, limit)
        {
            return self.emit_dispatch(ops, &dispatch, try_source_start);
        }
        let match_kind = if star {
            "CHECK_EG_MATCH"
        } else {
            "CHECK_EXC_MATCH"
        };
        if let Some(matched) = ops[handler + 1..]
            .iter()
            .position(|op| op.name == match_kind)
            .map(|n| handler + 1 + n)
        {
            let type_start = handler + if star { 3 } else { 1 };
            let kind = self.fragment(ops, type_start, matched, 0)?.expression()?;
            let pop = ops[matched + 1..]
                .iter()
                .position(|op| op.name == "POP_TOP" || op.name.starts_with("STORE_"))
                .map(|n| matched + 1 + n)
                .ok_or("missing exception binding")?;
            let clause = if ops[pop].name == "POP_TOP" {
                kind
            } else {
                let names = if matches!(ops[pop].name, "STORE_NAME" | "STORE_GLOBAL") {
                    self.code.names
                } else {
                    self.code.locals
                };
                format!("{kind} as {}", self.name(names, ops[pop].arg)?)
            };
            let body_end = if star {
                let first = ops.get(pop + 1).ok_or("empty exception handler")?.offset;
                bc.handlers
                    .iter()
                    .find(|h| h.start == first)
                    .and_then(|h| index(h.end).ok())
            } else if let Some(end) = self.raising_handler_end(ops, matched, pop) {
                Some(end)
            } else {
                ops[pop + 1..]
                    .iter()
                    .position(|op| op.name == "POP_EXCEPT")
                    .map(|n| pop + 1 + n)
            }
            .ok_or("unsupported exception handler extent")?;
            let suite = self.fragment(ops, pop + 1, body_end, 0)?.suite()?;
            self.block(
                &format!("except{} {clause}:", if star { "*" } else { "" }),
                &suite,
            )?;
            let join = ops
                .get(body_end + 1)
                .and_then(|op| op.jump)
                .and_then(|offset| index(offset).ok())
                .filter(|&idx| idx >= end && idx < handler)
                .unwrap_or(end);
            if join > end {
                let alternate = self.fragment(ops, end, join, 0)?.suite()?;
                self.block("else:", &alternate)?;
            }
            return self.region(ops, join, normal_end, 0);
        }
        if ops.get(handler + 1).is_some_and(|op| op.name == "POP_TOP") {
            let (_, cleanup) = super::dispatch::unwind(ops, &self.flow, handler, entry.depth + 1)
                .ok_or("missing bare exception cleanup")?;
            let stop = if ops[cleanup - 1].name == "RAISE_VARARGS"
                && self
                    .flow
                    .handler(cleanup - 1)
                    .is_some_and(|h| h.target == ops[cleanup].offset && h.lasti)
            {
                cleanup
            } else {
                (handler + 2..cleanup)
                    .find(|&i| ops[i].name == "POP_EXCEPT")
                    .ok_or("missing bare exception exit")?
            };
            let suite = self.fragment(ops, handler + 2, stop, 0)?.suite()?;
            self.block("except:", &suite)?;
            return self.region(ops, end, normal_end, 0);
        }
        let last = ops[handler + 1..]
            .iter()
            .position(|op| op.name == "RERAISE")
            .map(|n| handler + 1 + n)
            .ok_or("unsupported finally exit")?;
        let cleanup = self.fragment(ops, handler + 1, last, 0)?.suite()?;
        // The normal copy can start after a guard's tracing NOP. Local loads
        // can also be fused differently on success and during unwinding.
        let start = end + usize::from(ops[end].name == "NOP");
        let next = self
            .finally_copy_end(
                ops,
                start..handler,
                handler + 1..last,
                super::finalizers::CopyExit::Fallthrough,
            )
            .ok_or("unsupported copied finally body")?;
        self.block("finally:", &cleanup)?;
        if returns {
            if next + 1 != handler || ops[next].name != "RETURN_VALUE" {
                return Err("unsupported return through finally".into());
            }
            Ok(())
        } else {
            self.region(ops, next, normal_end, 0)
        }
    }

    /// A handler that always raises has no normal `POP_EXCEPT`. Its failed
    /// match jumps to `RERAISE`. A bound exception also has an out-of-line
    /// cleanup that clears the name on the way out.
    fn raising_handler_end(
        &self,
        ops: &[Instruction],
        matched: usize,
        binding: usize,
    ) -> Option<usize> {
        let jump = matched + 1;
        let failed = self.flow.jump(jump)?;
        if ops[jump].name != "POP_JUMP_IF_FALSE"
            || failed <= binding + 1
            || (ops[failed].name, ops[failed].arg) != ("RERAISE", 0)
        {
            return None;
        }
        let end = if ops[binding].name == "POP_TOP" {
            failed
        } else {
            self.exception_name_cleanup(ops, binding, failed)?
        };
        (end > binding + 1 && ops[end - 1].name == "RAISE_VARARGS").then_some(end)
    }

    /// A conditional transfer through finally has two normal copies of the
    /// cleanup and an exceptional copy. Match all three before lifting the
    /// break out of the normal copy.
    pub(super) fn finally_break(
        &mut self,
        ops: &[Instruction],
        predicate: &Predicate,
        handler: Handler,
        level: usize,
        end: usize,
    ) -> Result<usize, String> {
        let (taken, fallthrough) = (predicate.target, predicate.suite);
        let exceptional = boundary(ops, handler.target)?;
        let unwind = self
            .flow
            .handler(exceptional)
            .filter(|h| h.lasti)
            .ok_or("missing finally unwind handler")?;
        let stop = boundary(ops, unwind.end)?;
        if ops[exceptional].name != "PUSH_EXC_INFO"
            || stop <= exceptional + 1
            || ops[stop - 1].name != "RERAISE"
        {
            return Err("unsupported finally cleanup".into());
        }
        let length = stop - exceptional - 2;
        let cleanup = self
            .fragment(ops, exceptional + 1, stop - 1, level)?
            .suite()?;
        let mut normal_copy = |start: usize, limit: usize| -> Result<usize, String> {
            let start = start + usize::from(ops[start].name == "NOP");
            let after = start + length;
            if after > limit || self.fragment(ops, start, after, level)?.suite()? != cleanup {
                return Err("distinct finally cleanup paths".into());
            }
            Ok(after)
        };
        let transfer = normal_copy(fallthrough, taken)?;
        let next = normal_copy(taken, end)?;
        if self.fragment(ops, transfer, taken, level)?.suite()?.trim() != "break" {
            return Err("unsupported transfer through finally".into());
        }
        self.block(
            "try:",
            &format!("if {}:\n    break\n", predicate.test(true)),
        )?;
        self.block("finally:", &cleanup)?;
        Ok(next)
    }
}
