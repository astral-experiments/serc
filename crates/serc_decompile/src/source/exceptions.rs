//! The exception table records protected intervals and unwind targets. CPython
//! duplicates cleanup on normal paths and places the exceptional copy out of line.
use super::conditionals::Predicate;
use super::{Body, Bytecode, Instruction, Value};
use serc_bytecode::instructions::Handler;
use serc_bytecode::target::operands::SpecialMethod;

fn boundary(ops: &[Instruction], offset: usize) -> Result<usize, String> {
    ops.binary_search_by_key(&offset, |op| op.offset)
        .map_err(|_| "invalid exception boundary".into())
}

fn is_manager_handler(ops: &[Instruction], entry: &Handler) -> Result<bool, String> {
    let target = boundary(ops, entry.target)?;
    Ok(ops[target].name == "PUSH_EXC_INFO"
        && ops
            .get(target + 1)
            .is_some_and(|op| op.name == "WITH_EXCEPT_START"))
}

/// Locate the implicit manager protocol immediately before a protected binding.
fn manager_entry(ops: &[Instruction]) -> Result<usize, String> {
    match ops {
        [.., before] if before.name == "BEFORE_WITH" => Ok(ops.len() - 1),
        [.., copy, exit, swap2, swap3, enter, call]
            if (copy.name, copy.arg) == ("COPY", 1)
                && (exit.name, exit.arg)
                    == ("LOAD_SPECIAL", SpecialMethod::__EXIT__.argument())
                && (swap2.name, swap2.arg) == ("SWAP", 2)
                && (swap3.name, swap3.arg) == ("SWAP", 3)
                && (enter.name, enter.arg)
                    == ("LOAD_SPECIAL", SpecialMethod::__ENTER__.argument())
                && (call.name, call.arg) == ("CALL", 0) =>
        {
            Ok(ops.len() - 6)
        }
        _ => Err("unsupported manager entry".into()),
    }
}

impl Body<'_, '_, '_> {
    pub(super) fn protected(&mut self, bc: &Bytecode) -> Result<(), String> {
        let entry = bc.handlers[0];
        let ops = &bc.ops;
        let index = |offset| boundary(ops, offset);
        let start = index(entry.start)?;
        let end = index(entry.end)?;
        let handler = index(entry.target)?;
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
            return self.with_statement(bc);
        }
        if entry.depth > 0 && !entry.lasti {
            // A protected condition inside a loop keeps the iterator on the
            // stack. Its exceptional cleanup is out of line.
            return self.region(ops, 0, handler, 0);
        }
        if entry.depth != 0 || entry.lasti {
            return Err(format!("unsupported handler depth at byte {}", entry.start));
        }
        self.region(ops, 0, start, 0)?;
        let (suite, returns) = self.fragment(ops, start, end, 0)?.pending_return()?;
        self.block("try:", &suite)?;
        let star = ops
            .get(handler + 1)
            .is_some_and(|op| op.name == "BUILD_LIST");
        if !star
            && returns
            && end + 1 == handler
            && ops[end].name == "RETURN_VALUE"
            && ops.get(handler + 1).is_some_and(|_| {
                self.flow
                    .block(handler + 1)
                    .ops
                    .clone()
                    .any(|i| ops[i].name == "CHECK_EXC_MATCH")
            })
        {
            return self.returning_handlers(ops, handler);
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
                let names = if ops[pop].name == "STORE_NAME" {
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
            return self.region(ops, join, handler, 0);
        }
        if ops.get(handler + 1).is_some_and(|op| op.name == "POP_TOP") {
            let stop = ops[handler + 2..]
                .iter()
                .position(|op| op.name == "POP_EXCEPT")
                .map(|n| handler + 2 + n)
                .ok_or("missing bare exception cleanup")?;
            let suite = self.fragment(ops, handler + 2, stop, 0)?.suite()?;
            self.block("except:", &suite)?;
            return self.region(ops, end, handler, 0);
        }
        let last = ops[handler + 1..]
            .iter()
            .position(|op| op.name == "RERAISE")
            .map(|n| handler + 1 + n)
            .ok_or("unsupported finally exit")?;
        let cleanup = self.fragment(ops, handler + 1, last, 0)?.suite()?;
        // Find the corresponding normal copy by the number of logical
        // instructions in the exceptional copy. The whole-module verification
        // checks both paths, including exception regions and the cleanup code.
        let next = end + (last - handler - 1);
        if next > handler {
            return Err("unsupported copied finally body".into());
        }
        self.block("finally:", &cleanup)?;
        if returns {
            if next + 1 != handler || ops[next].name != "RETURN_VALUE" {
                return Err("unsupported return through finally".into());
            }
            Ok(())
        } else {
            self.region(ops, next, handler, 0)
        }
    }

    /// Follow the failed-match edge to each subsequent handler. When the try
    /// returns, CPython can duplicate the continuation's return after `POP_EXCEPT`
    /// in every successful handler, leaving no jump to the original return.
    fn returning_handlers(&mut self, ops: &[Instruction], handler: usize) -> Result<(), String> {
        let mut cursor = handler + 1;
        let mut continuation = None;
        while ops.get(cursor).is_some_and(|op| op.name != "RERAISE") {
            let check = self
                .flow
                .block(cursor)
                .ops
                .clone()
                .find(|&i| ops[i].name == "CHECK_EXC_MATCH")
                .ok_or("unsupported exception type")?;
            let jump = check + 1;
            let next = self
                .flow
                .jump(jump)
                .filter(|&next| next > jump && ops[jump].name == "POP_JUMP_IF_FALSE")
                .ok_or("unsupported exception dispatch")?;
            let kind = self.fragment(ops, cursor, check, 0)?.expression()?;
            let binding = jump + 1 + usize::from(ops[jump + 1].name == "NOT_TAKEN");
            if binding >= next || ops[binding].name != "POP_TOP" {
                return Err("unsupported returning exception binding".into());
            }
            let pop = (binding + 1..next)
                .find(|&i| ops[i].name == "POP_EXCEPT")
                .ok_or("missing exception cleanup")?;
            let suite = self.fragment(ops, binding + 1, pop, 0)?.suite()?;
            self.block(&format!("except {kind}:"), &suite)?;
            if !matches!(ops[next - 1].name, "RETURN_CONST" | "RETURN_VALUE") {
                return Err("unsupported exception continuation".into());
            }
            let tail = self.fragment(ops, pop + 1, next, 0)?.suite()?;
            if continuation
                .as_ref()
                .is_some_and(|previous| previous != &tail)
            {
                return Err("distinct handler continuations".into());
            }
            continuation = Some(tail);
            cursor = next;
        }
        for line in continuation.ok_or("missing exception handler")?.lines() {
            self.line(line)?;
        }
        Ok(())
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

    pub(super) fn manager_item(
        &mut self,
        manager: String,
        ops: &[Instruction],
        start: usize,
        end: usize,
    ) -> Result<(String, usize, Vec<Value>), String> {
        if ops.get(start).is_some_and(|op| op.name == "POP_TOP") {
            return Ok((manager, start + 1, Vec::new()));
        }
        let binding = self.binding(ops, start, end)?;
        Ok((
            format!("{manager} as {}", binding.target),
            binding.next,
            binding.seed,
        ))
    }

    fn with_statement(&mut self, bc: &Bytecode) -> Result<(), String> {
        let ops = &bc.ops;
        let mut ranges = bc.handlers.iter().peekable();
        let mut cursor = 0;
        let mut normal_end = ops.len();
        let depth = bc.handlers[0].depth;
        while cursor < normal_end {
            // Skip ranges that resume outer protection during the previous
            // group's cleanup. Only a subsequent independent manager at the
            // same depth starts another statement.
            let Some(entry) = ranges.find(|range| range.start >= ops[cursor].offset) else {
                break;
            };
            if entry.depth != depth
                || !entry.lasti
                || boundary(ops, entry.start)? >= normal_end
                || !is_manager_handler(ops, entry)?
            {
                break;
            }
            let Some(next) =
                self.manager_group(ops, cursor, entry, &mut ranges, &mut normal_end)?
            else {
                return Ok(());
            };
            cursor = next;
        }
        self.region(ops, cursor, normal_end, 0)
    }

    /// Recover one with statement, including adjacent items in its header.
    /// Return the normal continuation, or None for a return through the managers.
    fn manager_group<'a>(
        &mut self,
        ops: &[Instruction],
        cursor: usize,
        mut entry: &'a Handler,
        ranges: &mut std::iter::Peekable<std::slice::Iter<'a, Handler>>,
        normal_end: &mut usize,
    ) -> Result<Option<usize>, String> {
        let mut managers = Vec::new();
        let mut previous_body = None;
        let mut seed = Vec::new();
        let (start, end) = loop {
            let start = boundary(ops, entry.start)?;
            let end = boundary(ops, entry.end)?;
            let handler = boundary(ops, entry.target)?;
            *normal_end = (*normal_end).min(handler);
            if start >= end || end > *normal_end {
                return Err("unsupported manager layout".into());
            }
            let enter = manager_entry(&ops[..start])?;
            let manager = if let Some(previous) = previous_body {
                self.seeded(ops, previous, enter, 0, std::mem::take(&mut seed))?
                    .expression()?
            } else {
                self.region(ops, cursor, enter, 0)?;
                self.expr()?
            };
            let (item, body_start, loaded) = self.manager_item(manager, ops, start, end)?;
            managers.push(item);
            seed = loaded;

            // Each additional item starts where the enclosing protected range
            // ends. The enclosing handler resumes protection during inner cleanup.
            if let Some(&inner) = ranges.peek()
                && inner.start == entry.end
                && inner.depth > entry.depth
                && inner.lasti
                && is_manager_handler(ops, inner)?
            {
                ranges.next();
                previous_body = Some(body_start);
                entry = inner;
            } else {
                break (body_start, end);
            }
        };
        let (body, returns) = self.seeded(ops, start, end, 0, seed)?.pending_return()?;
        self.block(&format!("with {}:", managers.join(", ")), &body)?;

        let mut next = end;
        for _ in &managers {
            let exit = ops[next..*normal_end]
                .iter()
                .position(|op| op.name == "CALL")
                .map(|n| next + n)
                .ok_or("unsupported manager exit")?;
            if exit + 1 >= *normal_end || ops[exit + 1].name != "POP_TOP" {
                return Err("unsupported manager result".into());
            }
            next = exit + 2;
        }
        if returns {
            if ops.get(next).is_none_or(|op| op.name != "RETURN_VALUE") {
                return Err("unsupported return through with".into());
            }
            Ok(None)
        } else {
            Ok(Some(next))
        }
    }
}
