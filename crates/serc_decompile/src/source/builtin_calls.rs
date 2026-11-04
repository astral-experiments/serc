//! CPython inlines loops for calls to builtins on generators after checking
//! the callable's identity. The other arm calls the original name normally.
use super::{Body, CommonConstant, Instruction, Object, Value, comparisons::same_instructions};

pub(super) fn guarded_builtin(ops: &[Instruction]) -> Option<&'static str> {
    let [copy, constant, compare, branch, ..] = ops else {
        return None;
    };
    if (copy.name, copy.arg) != ("COPY", 1)
        || constant.name != "LOAD_COMMON_CONSTANT"
        || (compare.name, compare.arg) != ("IS_OP", 0)
        || branch.name != "POP_JUMP_IF_FALSE"
    {
        return None;
    }
    [
        (CommonConstant::BUILTIN_ALL, "all"),
        (CommonConstant::BUILTIN_ANY, "any"),
        (CommonConstant::BUILTIN_TUPLE, "tuple"),
        (CommonConstant::BUILTIN_LIST, "list"),
        (CommonConstant::BUILTIN_SET, "set"),
    ]
    .into_iter()
    .find_map(|(builtin, name)| (builtin.argument() == constant.arg).then_some(name))
}

impl Body<'_, '_, '_> {
    pub(super) fn optimized_generator(
        &mut self,
        ops: &[Instruction],
        guard: usize,
        name: &str,
    ) -> Result<Option<usize>, String> {
        let fallback = self.flow.target(guard)?;
        self.charge(fallback.saturating_sub(guard))?;
        Ok(self.generator_call(ops, guard, fallback, name))
    }

    fn generator_call(
        &self,
        ops: &[Instruction],
        guard: usize,
        fallback: usize,
        name: &str,
    ) -> Option<usize> {
        let short_circuit = matches!(name, "all" | "any");
        if (matches!(name, "list" | "set") && self.m.version < super::PythonVersion::Py315)
            || !matches!(self.stack.last(), Some(Value::Expr(value)) if value == name)
            || ops.get(fallback)?.name != "PUSH_NULL"
        {
            return None;
        }
        let fast = ops.get(guard + 1..fallback)?;
        let [not_taken, discard, rest @ ..] = fast else {
            return None;
        };
        if not_taken.name != "NOT_TAKEN"
            || discard.name != "POP_TOP"
            || !self
                .flow
                .blocks_in(guard + 1..fallback)
                .all(|block| self.flow.entered_from(block.ops.start, guard..fallback))
        {
            return None;
        }
        let it = rest.iter().position(|op| op.name == "FOR_ITER")?;
        let iterator = guard + 3 + it;
        let argument = if short_circuit {
            rest.get(..it)?
        } else {
            let (build, argument) = rest.get(..it)?.split_first()?;
            let build_name = if name == "set" {
                "BUILD_SET"
            } else {
                "BUILD_LIST"
            };
            if (build.name, build.arg) != (build_name, 0) {
                return None;
            }
            argument
        };
        // 3.15 uses a second iterator slot. It belongs to the fast loop.
        let argument = if self.m.version == super::PythonVersion::Py315 {
            let (last, arg) = argument.split_last()?;
            (last.name == "PUSH_NULL").then_some(arg)?
        } else {
            argument
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
        let exhausted = self.flow.jump(iterator)?;
        if (ops.get(exhausted)?.name, ops.get(exhausted + 1)?.name) != ("END_FOR", "POP_ITER") {
            return None;
        }
        let join = call + 1;
        if !short_circuit {
            let [append, back] = ops.get(iterator + 1..exhausted)? else {
                return None;
            };
            let append_name = if name == "set" {
                "SET_ADD"
            } else {
                "LIST_APPEND"
            };
            let depth = if self.m.version == super::PythonVersion::Py315 {
                3
            } else {
                2
            };
            if (append.name, append.arg) != (append_name, depth)
                || back.name != "JUMP_BACKWARD"
                || self.flow.jump(iterator + 2) != Some(iterator)
            {
                return None;
            }
            let mut exit = exhausted + 2;
            if name == "tuple" {
                let convert = ops.get(exit)?;
                if (convert.name, convert.arg)
                    != (
                        "CALL_INTRINSIC_1",
                        serc_bytecode::target::operands::UnaryIntrinsic::LIST_TO_TUPLE.argument(),
                    )
                {
                    return None;
                }
                exit += 1;
            }
            return self
                .builtin_exit(ops, exit, fallback, join)
                .then_some(fallback);
        }
        let all = name == "all";
        let [truth, branch, not_taken, back, ..] = ops.get(iterator + 1..fallback)? else {
            return None;
        };
        let test = if all {
            "POP_JUMP_IF_FALSE"
        } else {
            "POP_JUMP_IF_TRUE"
        };
        let matched = iterator + 5;
        if (truth.name, branch.name, not_taken.name, back.name)
            != ("TO_BOOL", test, "NOT_TAKEN", "JUMP_BACKWARD")
            || self.flow.jump(iterator + 2) != Some(matched)
            || self.flow.jump(iterator + 4) != Some(iterator)
            || ops.get(matched)?.name != "POP_ITER"
            || self.result_bool(*ops.get(matched + 1)?) != Some(!all)
            || self.result_bool(*ops.get(exhausted + 2)?) != Some(all)
        {
            return None;
        }
        (self.builtin_exit(ops, matched + 2, exhausted, join)
            && self.builtin_exit(ops, exhausted + 3, fallback, join))
        .then_some(fallback)
    }

    // The fast result rejoins the ordinary call, or it executes an identical
    // copy of the terminal instructions following that call.
    fn builtin_exit(&self, ops: &[Instruction], start: usize, stop: usize, join: usize) -> bool {
        let Some(exit) = ops.get(start..stop) else {
            return false;
        };
        matches!(exit, [jump] if jump.name == "JUMP_FORWARD" && self.flow.jump(start) == Some(join))
            || (matches!(exit.last(), Some(op) if matches!(op.name, "RETURN_VALUE" | "RETURN_CONST"))
                && ops
                    .get(join..join + exit.len())
                    .is_some_and(|tail| same_instructions(exit, tail)))
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
