//! On failure, each subpattern discards its still-unmatched siblings. The
//! resulting destination must agree with the failure of its enclosing pattern.
use std::ops::Range;

use super::value_patterns::{after_test, header};
use super::{Body, Instruction, Object, comparisons::operator, identifier};

struct Pattern {
    text: String,
    next: usize,
    failure: Option<usize>,
}

struct ClassPattern<'m> {
    name: String,
    positional: usize,
    keywords: &'m [super::Id],
}

impl<'m> Body<'m, '_, '_> {
    fn pattern_failure(
        &mut self,
        ops: &[Instruction],
        mut at: usize,
        mut discard: usize,
    ) -> Option<usize> {
        loop {
            self.charge(1).ok()?;
            match ops.get(at)?.name {
                "JUMP_FORWARD" => {
                    let to = self.flow.jump(at)?;
                    if to <= at {
                        return None;
                    }
                    at = to;
                }
                "POP_TOP" if discard > 0 => {
                    at += 1;
                    discard -= 1;
                }
                _ => return (discard == 0).then_some(at),
            }
        }
    }

    // A short constant-return suite can be copied onto the success or failure
    // of the last OR pattern instead of leaving a forward jump. Compare the
    // complete exit, including a saved subject discard or wildcard tracing NOP.
    fn case_return(&self, ops: &[Instruction], start: usize) -> Option<Range<usize>> {
        let suite = start + usize::from(ops.get(start)?.name == "POP_TOP");
        let load = suite + usize::from(ops.get(suite)?.name == "NOP");
        ops.get(load)?;
        let end = self.flow.block(load).ops.end;
        let (at, _) = super::regions::constant_return(ops, end)?;
        (load == at).then_some(start..end)
    }

    fn same_case_exit(&self, ops: &[Instruction], left: usize, right: usize) -> bool {
        left == right
            || self
                .case_return(ops, left)
                .zip(self.case_return(ops, right))
                .is_some_and(|(a, b)| {
                    super::comparisons::same_instructions(&ops[a.clone()], &ops[b.clone()])
                        && a.zip(b)
                            .all(|(a, b)| self.flow.handler(a) == self.flow.handler(b))
                })
    }

    fn pattern_alternatives(
        &mut self,
        ops: &[Instruction],
        mut at: usize,
        level: usize,
    ) -> Option<Pattern> {
        let mut success = None;
        let mut alternatives = Vec::new();
        loop {
            let part = self.pattern_item(ops, at + 1, level + 1, false)?;
            let fail = self.pattern_failure(ops, part.failure?, 0)?;
            if fail <= part.next {
                return None;
            }
            let body = match ops.get(part.next)?.name {
                "JUMP_FORWARD" => {
                    let join = self.flow.jump(part.next)?;
                    if join <= fail || ops.get(join)?.name != "POP_TOP" {
                        return None;
                    }
                    join + 1
                }
                "POP_TOP" => {
                    let ret = self.case_return(ops, part.next + 1)?;
                    if ret.end > fail {
                        return None;
                    }
                    ret.start
                }
                _ => return None,
            };
            if success.is_some_and(|old| !self.same_case_exit(ops, old, body)) {
                return None;
            }
            success = Some(body);
            alternatives.push(part.text);
            if (ops[fail].name, ops[fail].arg) == ("COPY", 1) {
                at = fail;
                continue;
            }
            if alternatives.len() < 2 || ops[fail].name != "POP_TOP" {
                return None;
            }
            return Some(Pattern {
                text: alternatives.join(" | "),
                next: body,
                failure: Some(self.pattern_failure(ops, fail, 1)?),
            });
        }
    }

    fn sequence_shape(&self, ops: &[Instruction], at: usize) -> Option<(usize, usize)> {
        let branch = at + 1;
        if ops.get(branch)?.name != "POP_JUMP_IF_FALSE" {
            return None;
        }
        let length = after_test(ops, branch);
        let load = ops.get(length + 1)?;
        let count = match load.name {
            "LOAD_SMALL_INT" => load.arg,
            "LOAD_CONST" => {
                let id = self.item(self.code.constants, load.arg).ok()?;
                let Object::Int(value) = self.m.objects[id] else {
                    return None;
                };
                u32::try_from(value).ok()?
            }
            _ => return None,
        };
        let test = length + 3;
        let unpack = after_test(ops, test);
        (ops.get(length)?.name == "GET_LEN"
            && operator(*ops.get(length + 2)?, self.m.version) == Some("==")
            && ops.get(test)?.name == "POP_JUMP_IF_FALSE"
            && self.flow.jump(test)? == self.flow.jump(branch)?
            && (ops.get(unpack)?.name, ops[unpack].arg) == ("UNPACK_SEQUENCE", count))
            .then_some((branch, unpack))
    }

    fn class_shape(
        &mut self,
        ops: &[Instruction],
        at: usize,
        level: usize,
    ) -> Option<(ClassPattern<'m>, usize, usize)> {
        // The class expression is a name or an attribute path, immediately
        // followed by the constant tuple of keyword attribute names.
        if !matches!(
            ops.get(at)?.name,
            "LOAD_GLOBAL" | "LOAD_NAME" | "LOAD_FAST" | "LOAD_FAST_BORROW" | "LOAD_DEREF"
        ) {
            return None;
        }
        let mut keys = at + 1;
        while ops.get(keys)?.name == "LOAD_ATTR" && ops[keys].arg & 1 == 0 {
            keys += 1;
        }
        let match_class = keys + 1;
        let branch = match_class + 2;
        if ops[keys].name != "LOAD_CONST"
            || ops.get(match_class)?.name != "MATCH_CLASS"
            || (ops.get(match_class + 1)?.name, ops[match_class + 1].arg) != ("COPY", 1)
            || ops.get(branch)?.name != "POP_JUMP_IF_NONE"
        {
            return None;
        }
        let unpack = after_test(ops, branch);
        let names = self
            .m
            .tuple(self.item(self.code.constants, ops[keys].arg).ok()?)
            .ok()?;
        let positional = ops[match_class].arg as usize;
        if ops.get(unpack)?.name != "UNPACK_SEQUENCE"
            || ops[unpack].arg as usize != positional + names.len()
        {
            return None;
        }
        let name = self.pattern_expression(ops, at, keys, level).ok()?;
        Some((
            ClassPattern {
                name,
                positional,
                keywords: names,
            },
            branch,
            unpack,
        ))
    }

    fn pattern_item(
        &mut self,
        ops: &[Instruction],
        at: usize,
        level: usize,
        capture: bool,
    ) -> Option<Pattern> {
        self.charge(1).ok()?;
        if level > 32 {
            return None;
        }
        let op = ops.get(at)?;
        if (op.name, op.arg) == ("COPY", 1) {
            return self.pattern_alternatives(ops, at, level);
        }
        if capture && matches!(op.name, "STORE_FAST" | "STORE_NAME") {
            let table = if op.name == "STORE_FAST" {
                self.code.locals
            } else {
                self.code.names
            };
            return Some(Pattern {
                text: self.name(table, op.arg).ok()?.to_owned(),
                next: at + 1,
                failure: None,
            });
        }
        if let Some(test) = header(ops, at, self.m.version)
            && test.copies == 0
            && ops[test.branch].name != "POP_JUMP_IF_TRUE"
        {
            return Some(Pattern {
                text: self.pattern_operand(ops, &test, level).ok()?,
                next: after_test(ops, test.branch),
                failure: Some(self.flow.jump(test.branch)?),
            });
        }

        let mut class = None;
        let (branch, unpack) = if op.name == "MATCH_SEQUENCE" {
            self.sequence_shape(ops, at)?
        } else {
            let (pattern, branch, unpack) = self.class_shape(ops, at, level)?;
            class = Some(pattern);
            (branch, unpack)
        };
        let failure = self.pattern_failure(ops, self.flow.jump(branch)?, 1)?;
        let count = ops[unpack].arg as usize;
        let mut next = unpack + 1;
        let mut fields = Vec::new();
        for field in 0..count {
            let remaining = count - field - 1;
            let part = self.pattern_item(ops, next, level + 1, capture && remaining == 0)?;
            if let Some(fail) = part.failure {
                let child_exit = self.pattern_failure(ops, fail, remaining)?;
                if !self.same_case_exit(ops, child_exit, failure) {
                    return None;
                }
            }
            next = part.next;
            fields.push(
                if let Some(class) = &class
                    && field >= class.positional
                {
                    let name =
                        identifier(self.m.text(class.keywords[field - class.positional]).ok()?)
                            .ok()?;
                    format!("{name}={}", part.text)
                } else {
                    part.text
                },
            );
        }
        let fields = fields.join(", ");
        Some(Pattern {
            text: if let Some(class) = class {
                format!("{}({fields})", class.name)
            } else {
                format!("[{fields}]")
            },
            next,
            failure: Some(failure),
        })
    }

    pub(super) fn terminal_sequence_cases(
        &mut self,
        ops: &[Instruction],
        mut at: usize,
        level: usize,
    ) -> Option<Vec<(String, Range<usize>)>> {
        let mut arms = Vec::new();
        loop {
            let saved = (ops.get(at)?.name, ops[at].arg) == ("COPY", 1);
            let head = at + usize::from(saved);
            if ops.get(head)?.name != "MATCH_SEQUENCE" {
                return None;
            }
            let pattern = self.pattern_item(ops, head, level, true)?;
            let body = pattern.next + usize::from(saved);
            if saved && ops.get(pattern.next)?.name != "POP_TOP" {
                return None;
            }
            let next = pattern.failure?;
            if next <= body {
                return None;
            }
            let mut stop = self.case_return(ops, body).map_or(next, |ret| ret.end);
            if stop > next {
                return None;
            }
            while stop > body && ops[stop - 1].name == "POP_TOP" {
                stop -= 1;
            }
            if stop == body
                || !matches!(
                    ops[stop - 1].name,
                    "RETURN_CONST" | "RETURN_VALUE" | "RAISE_VARARGS"
                )
                || self.flow.blocks_covering(body..stop).any(|block| {
                    [
                        self.flow.jump(block.ops.end - 1),
                        self.flow.fallthrough(block),
                    ]
                    .into_iter()
                    .flatten()
                    .any(|to| !(body..stop).contains(&to))
                })
            {
                return None;
            }
            arms.push((pattern.text, body..stop));
            if !saved {
                arms.push(("_".to_owned(), next..ops.len()));
                return Some(arms);
            }
            at = next;
        }
    }
}
