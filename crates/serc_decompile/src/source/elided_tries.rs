//! A try suite containing only `pass` can lose its protected interval entirely.
//! The exceptional copy remains out of line with its own lasti cleanup entries.
use super::{Body, Bytecode, Instruction, finalizers};
use serc_bytecode::target::operands::BinaryIntrinsic;

impl Body<'_, '_, '_> {
    pub(super) fn elided_tries(&mut self, bc: &Bytecode, normal_end: usize) -> Result<(), String> {
        let ops = &bc.ops;
        let mut cursor = 0;
        let mut start = normal_end;
        while start < ops.len() {
            let handler = self
                .flow
                .handler(start)
                .filter(|h| h.start == ops[start].offset && h.depth == 1 && h.lasti)
                .ok_or("missing cleanup for elided try")?;
            let cleanup = ops
                .binary_search_by_key(&handler.target, |op| op.offset)
                .map_err(|_| "invalid elided try cleanup")?;
            if ops[start].name != "PUSH_EXC_INFO"
                || self.flow.reachable(start)
                || cleanup <= start + 1
                || !matches!(ops.get(cleanup..),Some([copy,pop,raise,..]) if (copy.name,copy.arg)==("COPY",3) && pop.name=="POP_EXCEPT" && (raise.name,raise.arg)==("RERAISE",1))
            {
                return Err("unsupported elided try cleanup".into());
            }
            let lines = self.instruction_lines(ops)?;
            let clause_line = lines[start + 1..cleanup]
                .iter()
                .copied()
                .flatten()
                .next()
                .ok_or("missing elided handler location")?;
            let marker = (cursor..normal_end)
                .take_while(|&i| lines[i].is_none_or(|line| line < clause_line))
                .filter(|&i| ops[i].name == "NOP")
                .last()
                .ok_or("missing elided try location")?;
            self.region(ops, cursor, marker, 0)?;
            self.block("try:", "")?;
            cursor = marker + 1;
            // With an empty try, a finally suite is still copied onto both
            // paths. Its exceptional copy ends by reraising the exception.
            let copy = start + 1..cleanup - 1;
            if (ops[cleanup - 1].name, ops[cleanup - 1].arg) == ("RERAISE", 0)
                && finalizers::owns_cleanup(ops, self, start..cleanup, handler)
                && !copy.is_empty()
                && let Some(after) = self.finally_copy_end(
                    ops,
                    cursor..normal_end,
                    copy,
                    finalizers::CopyExit::Fallthrough,
                )
            {
                let suite = self.fragment(ops, cursor, after, 0)?.suite()?;
                self.block("finally:", &suite)?;
                cursor = after;
            } else {
                self.elided_handlers(ops, start + 1, cleanup)?;
            }
            start = cleanup + 3;
        }
        self.region(ops, cursor, normal_end, 0)
    }

    fn elided_handlers(
        &mut self,
        ops: &[Instruction],
        mut at: usize,
        cleanup: usize,
    ) -> Result<(), String> {
        if matches!(ops.get(at..cleanup), Some([list, copy, ..]) if
            (list.name, list.arg) == ("BUILD_LIST", 0) && (copy.name, copy.arg) == ("COPY", 2))
        {
            return self.elided_group_handler(ops, at + 2, cleanup);
        }
        while at < cleanup {
            if (ops[at].name, ops[at].arg) == ("RERAISE", 0) {
                return Ok(());
            }
            let bare = ops[at].name == "POP_TOP";
            let (mut clause, binding, next) = if bare {
                ("except".to_owned(), at, cleanup)
            } else {
                let matched = (at..self.flow.block(at).ops.end.min(cleanup))
                    .find(|&i| ops[i].name == "CHECK_EXC_MATCH")
                    .ok_or("unsupported elided exception clause")?;
                let branch = matched + 1;
                if branch >= cleanup || ops[branch].name != "POP_JUMP_IF_FALSE" {
                    return Err("unsupported elided exception test".into());
                }
                let kind = self.fragment(ops, at, matched, 0)?.expression()?;
                (
                    format!("except {kind}"),
                    branch + 1 + usize::from(ops[branch + 1].name == "NOT_TAKEN"),
                    self.flow.target(branch)?,
                )
            };
            if binding >= next || next > cleanup {
                return Err("invalid elided handler extent".into());
            }
            let (body, seed) = if ops[binding].name == "POP_TOP" {
                (binding + 1, Vec::new())
            } else {
                let bound = self.binding(ops, binding, next)?;
                clause.push_str(" as ");
                clause.push_str(&bound.target);
                (bound.next, bound.seed)
            };
            let pop = (body..next)
                .find(|&i| ops[i].name == "POP_EXCEPT")
                .ok_or("missing elided handler exit")?;
            let suite = self.seeded(ops, body, pop, 0, seed)?.suite()?;
            self.block(&format!("{clause}:"), &suite)?;
            at = next;
        }
        Ok(())
    }

    fn elided_group_handler(
        &mut self,
        ops: &[Instruction],
        type_start: usize,
        cleanup: usize,
    ) -> Result<(), String> {
        let matched = (type_start..cleanup)
            .find(|&i| ops[i].name == "CHECK_EG_MATCH")
            .ok_or("missing elided exception group match")?;
        let branch = matched + 2;
        if !matches!(ops.get(matched + 1..cleanup), Some([copy, jump, ..]) if
            (copy.name, copy.arg) == ("COPY", 1) && jump.name == "POP_JUMP_IF_NONE")
        {
            return Err("unsupported elided exception group test".into());
        }
        let taken = self.flow.target(branch)?;
        let fall = branch + 1 + usize::from(ops[branch + 1].name == "NOT_TAKEN");
        // With a single empty clause, both paths discard the subgroup and
        // append the remainder before the same PREP_RERAISE_STAR. A handler
        // body or another type test cannot be skipped on either path.
        let merge = |i: usize| {
            if i >= cleanup || ops[i].name != "POP_TOP" {
                return None;
            }
            let (join, appended) =
                super::exception_groups::next_match(ops, &self.flow, i + 1, cleanup)?;
            (appended
                && (ops[join].name, ops[join].arg)
                    == (
                        "CALL_INTRINSIC_2",
                        BinaryIntrinsic::PREP_RERAISE_STAR.argument(),
                    ))
                .then_some(join)
        };
        if merge(taken).is_none_or(|join| merge(fall) != Some(join)) {
            return Err("unsupported elided exception group suite".into());
        }
        let kind = self.fragment(ops, type_start, matched, 0)?.expression()?;
        self.block(&format!("except* {kind}:"), "")
    }
}
