//! CPython inlines a loop for `all(genexpr)` and `any(genexpr)` after checking
//! the callable's identity. The other arm calls the original name normally.
use super::{Body, CommonConstant, Instruction, Object, Value, comparisons::same_instructions};

pub(super) fn truth_builtin(arg: u32) -> Option<bool> {
    if arg == CommonConstant::BUILTIN_ALL.argument() {
        Some(true)
    } else if arg == CommonConstant::BUILTIN_ANY.argument() {
        Some(false)
    } else {
        None
    }
}

impl Body<'_, '_, '_> {
    pub(super) fn optimized_truth_generator(
        &mut self,
        ops: &[Instruction],
        guard: usize,
        all: bool,
    ) -> Result<Option<usize>, String> {
        let fallback = self.flow.target(guard)?;
        self.charge(fallback.saturating_sub(guard))?;
        Ok(self.truth_generator_call(ops, guard, fallback, all))
    }

    fn truth_generator_call(
        &self,
        ops: &[Instruction],
        guard: usize,
        fallback: usize,
        all: bool,
    ) -> Option<usize> {
        let name = if all { "all" } else { "any" };
        if !matches!(self.stack.last(), Some(Value::Expr(value)) if value == name)
            || ops.get(fallback)?.name != "PUSH_NULL"
        {
            return None;
        }
        let fast = ops.get(guard + 1..fallback)?;
        let [not_taken, discard, rest @ ..] = fast else {
            return None;
        };
        if not_taken.name != "NOT_TAKEN" || discard.name != "POP_TOP" {
            return None;
        }
        let it = rest.iter().position(|op| op.name == "FOR_ITER")?;
        let iterator = guard + 3 + it;
        let argument = rest.get(..it)?;
        // 3.15 uses a second iterator slot. It belongs to the fast loop.
        let argument = match argument {
            [arg @ .., last] if last.name == "PUSH_NULL" => arg,
            other => other,
        };
        let call = fallback + 1 + argument.len();
        if !matches!(argument.last(), Some(op) if (op.name, op.arg) == ("CALL", 0))
            || !argument.iter().any(|op| {
                op.name == "LOAD_CONST"
                    && self
                        .item(self.code.constants, op.arg)
                        .ok()
                        .and_then(|id| self.m.code(id).ok())
                        .and_then(|code| self.m.text(code.name).ok())
                        == Some("<genexpr>")
            })
            || !same_instructions(argument, ops.get(fallback + 1..call)?)
            || (ops.get(call)?.name, ops[call].arg) != ("CALL", 1)
        {
            return None;
        }
        let [truth, branch, not_taken, back, ..] = ops.get(iterator + 1..fallback)? else {
            return None;
        };
        let test = if all {
            "POP_JUMP_IF_FALSE"
        } else {
            "POP_JUMP_IF_TRUE"
        };
        let matched = iterator + 5;
        let exhausted = self.flow.jump(iterator)?;
        if (truth.name, branch.name, not_taken.name, back.name)
            != ("TO_BOOL", test, "NOT_TAKEN", "JUMP_BACKWARD")
            || self.flow.jump(iterator + 2) != Some(matched)
            || self.flow.jump(iterator + 4) != Some(iterator)
            || ops.get(matched)?.name != "POP_ITER"
            || self.result_bool(*ops.get(matched + 1)?) != Some(!all)
            || ops.get(exhausted)?.name != "END_FOR"
            || ops.get(exhausted + 1)?.name != "POP_ITER"
            || self.result_bool(*ops.get(exhausted + 2)?) != Some(all)
        {
            return None;
        }
        let join = call + 1;
        // Both results must rejoin the ordinary call, or contain an identical
        // copy of its terminal instructions. Then the fallback alone recovers
        // the source, including the lookup of a possibly shadowed builtin.
        for (start, stop) in [(matched + 2, exhausted), (exhausted + 3, fallback)] {
            let exit = ops.get(start..stop)?;
            if matches!(exit, [jump] if jump.name == "JUMP_FORWARD" && self.flow.jump(start) == Some(join))
            {
                continue;
            }
            if !matches!(exit.last(), Some(op) if matches!(op.name, "RETURN_VALUE" | "RETURN_CONST"))
                || !same_instructions(exit, ops.get(join..join + exit.len())?)
            {
                return None;
            }
        }
        Some(fallback)
    }

    fn result_bool(&self, op: Instruction) -> Option<bool> {
        match op.name {
            "LOAD_CONST" => match self.m.objects[self.item(self.code.constants, op.arg).ok()?] {
                Object::Bool(value) => Some(value),
                _ => None,
            },
            "LOAD_COMMON_CONSTANT" if op.arg == CommonConstant::TRUE.argument() => Some(true),
            "LOAD_COMMON_CONSTANT" if op.arg == CommonConstant::FALSE.argument() => Some(false),
            _ => None,
        }
    }
}
