//! Recognize the two exits of a capture pattern. A failed mapping key or
//! length test can discard different numbers of temporaries on its way out.
use std::fmt::Write as _;

use super::{Body, Instruction, definitions::tuple_items, identifier};

fn find(ops: &[Instruction], start: usize, end: usize, name: &str) -> Result<usize, String> {
    ops.get(start..end)
        .and_then(|slice| slice.iter().position(|op| op.name == name))
        .map(|n| start + n)
        .ok_or_else(|| format!("missing {name} in match"))
}

fn target(ops: &[Instruction], i: usize, end: usize) -> Result<usize, String> {
    ops[i]
        .jump
        .and_then(|offset| ops.binary_search_by_key(&offset, |op| op.offset).ok())
        .filter(|&index| index > i && index < end)
        .ok_or_else(|| "invalid match exit".into())
}

fn case(out: &mut String, pattern: &str, suite: &str) {
    writeln!(out, "case {pattern}:").unwrap();
    for line in suite.lines() {
        writeln!(out, "    {line}").unwrap();
    }
    if suite.is_empty() {
        out.push_str("    pass\n");
    }
}

impl Body<'_, '_, '_> {
    pub(super) fn match_cases(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let copied = ops[i].name == "COPY";
        let head = i + usize::from(copied);
        let op = ops[head];
        let mut class = None;
        if op.name == "MATCH_CLASS" {
            let keys = self.expr()?;
            class = Some((self.expr()?, tuple_items(&keys)?));
        }
        let subject = self.expr()?;
        let branch = find(
            ops,
            head + 1,
            end,
            if class.is_some() {
                "POP_JUMP_IF_NONE"
            } else {
                "POP_JUMP_IF_FALSE"
            },
        )?;
        let fail = target(ops, branch, end)?;
        let unpack = find(ops, branch + 1, fail, "UNPACK_SEQUENCE")?;
        let mut start = unpack + 1;
        let mut bindings = Vec::new();
        while bindings.len() < ops[unpack].arg as usize && start < fail {
            let store = ops[start];
            match store.name {
                "STORE_FAST" | "STORE_NAME" => {
                    let table = if store.name == "STORE_FAST" {
                        self.code.locals
                    } else {
                        self.code.names
                    };
                    bindings.push(self.name(table, store.arg)?.to_owned());
                }
                "STORE_FAST_STORE_FAST" => {
                    for index in [store.arg >> 4, store.arg & 15] {
                        bindings.push(self.name(self.code.locals, index)?.to_owned());
                    }
                }
                _ => return Err("unsupported nested match pattern".into()),
            }
            start += 1;
        }
        if bindings.len() != ops[unpack].arg as usize {
            return Err("invalid match captures".into());
        }
        let mut body_end = fail;
        let mut pattern = if let Some((ty, keys)) = class {
            let positional = op.arg as usize;
            if positional + keys.len() != bindings.len() {
                return Err("invalid match class arity".into());
            }
            for (binding, key) in bindings[positional..].iter_mut().zip(keys) {
                let key = super::definitions::string_value(&key)?;
                *binding = format!("{}={binding}", identifier(&key)?);
            }
            format!("{ty}({})", bindings.join(", "))
        } else if op.name == "MATCH_MAPPING" {
            let keys = find(ops, branch + 1, unpack, "MATCH_KEYS")?;
            let key_op = ops[keys - 1];
            if key_op.name != "LOAD_CONST" {
                return Err("unsupported mapping key expression".into());
            }
            let id = self.item(self.code.constants, key_op.arg)?;
            let keys = self.m.tuple(id)?;
            if keys.len() != bindings.len() {
                return Err("invalid mapping captures".into());
            }
            let mut pairs = Vec::new();
            for (&key, binding) in keys.iter().zip(bindings) {
                pairs.push(format!(
                    "{}: {binding}",
                    super::literal(self.m, key, 0, &mut self.work.budget)?
                ));
            }
            body_end = target(ops, find(ops, branch + 1, unpack, "POP_JUMP_IF_NONE")?, end)?;
            format!("{{{}}}", pairs.join(", "))
        } else {
            format!("[{}]", bindings.join(", "))
        };
        if copied {
            let discard = find(ops, start, body_end, "POP_TOP")?;
            if let Ok(test) = find(ops, start, discard, "POP_JUMP_IF_FALSE") {
                let pred_end = test - usize::from(test > start && ops[test - 1].name == "TO_BOOL");
                let guard = self.fragment(ops, start, pred_end, level)?.expression()?;
                write!(pattern, " if {guard}").unwrap();
            }
            start = discard + 1;
        } else {
            while start < body_end && ops[start].name == "POP_TOP" {
                start += 1;
            }
        }
        let mut cases = String::new();
        case(
            &mut cases,
            &pattern,
            &self.fragment(ops, start, body_end, level)?.suite()?,
        );
        let mut next = fail + 1;
        if copied {
            let compare = find(ops, next, end, "COMPARE_OP")?;
            let value = self.fragment(ops, next, compare, level)?.expression()?;
            let test = find(ops, compare + 1, end, "POP_JUMP_IF_FALSE")?;
            let fallback = target(ops, test, end)?;
            let body = self.fragment(ops, test + 1, fallback, level)?.suite()?;
            case(&mut cases, &value, &body);
            next = fallback;
        }
        let fallback = self.fragment(ops, next, end, level)?.suite()?;
        if ops.get(next).is_some_and(|op| op.name != "NOP") && fallback.lines().count() == 1 {
            writeln!(cases, "case _: {}", fallback.trim_end()).unwrap();
        } else {
            case(&mut cases, "_", &fallback);
        }
        self.block(&format!("match {subject}:"), &cases)?;
        Ok(end)
    }
}
