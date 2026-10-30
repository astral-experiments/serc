//! The exception table records protected intervals and unwind targets. CPython
//! duplicates cleanup on normal paths and places the exceptional copy out of line.
use super::{Body, Bytecode, Instruction};

impl Body<'_, '_, '_> {
    pub(super) fn protected(&mut self, bc: &Bytecode) -> Result<(), String> {
        let entry = bc.handlers[0];
        let ops = &bc.ops;
        let index = |offset| {
            ops.binary_search_by_key(&offset, |op| op.offset)
                .map_err(|_| "invalid exception boundary")
        };
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
            return self.with_statement(ops, start, end, handler);
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

    fn with_statement(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        handler: usize,
    ) -> Result<(), String> {
        let enter = ops[..start]
            .iter()
            .rposition(|op| op.name == "BEFORE_WITH" || op.name == "COPY")
            .ok_or("unsupported manager entry")?;
        self.region(ops, 0, enter, 0)?;
        let manager = self.expr()?;
        let binding = match ops[start].name {
            "STORE_FAST" => format!(" as {}", self.name(self.code.locals, ops[start].arg)?),
            "STORE_NAME" => format!(" as {}", self.name(self.code.names, ops[start].arg)?),
            "POP_TOP" => String::new(),
            _ => return Err("unsupported with target".into()),
        };
        let (body, returns) = self.fragment(ops, start + 1, end, 0)?.pending_return()?;
        self.block(&format!("with {manager}{binding}:"), &body)?;
        let exit = ops[end..handler]
            .iter()
            .position(|op| op.name == "CALL")
            .map(|n| end + n)
            .ok_or("unsupported manager exit")?;
        if ops.get(exit + 1).is_none_or(|op| op.name != "POP_TOP") {
            return Err("unsupported manager result".into());
        }
        let next = exit + 2;
        if returns {
            if ops.get(next).is_none_or(|op| op.name != "RETURN_VALUE") {
                return Err("unsupported return through with".into());
            }
            Ok(())
        } else {
            self.region(ops, next, handler, 0)
        }
    }
}
