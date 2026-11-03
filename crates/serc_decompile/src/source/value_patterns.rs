//! Literal and singleton cases preserve the subject for a later case. An
//! additional copy for an `as` binding is discarded on pattern failure.
use std::fmt::Write as _;

use ruff_python_codegen::{Generator, Indentation};
use ruff_source_file::LineEnding;

use super::{
    Body, Instruction, PythonVersion, Value,
    comparisons::operator,
    patterns::{case, write_suite},
};

pub(super) struct Header {
    copies: usize,
    operand: usize,
    compare: usize,
    branch: usize,
}

pub(super) fn header(ops: &[Instruction], mut at: usize, version: PythonVersion) -> Option<Header> {
    let start = at;
    while at - start < 3
        && ops
            .get(at)
            .is_some_and(|op| (op.name, op.arg) == ("COPY", 1))
    {
        at += 1;
    }
    let copies = at - start;
    let operand = at;
    if ops.get(at)?.name == "POP_JUMP_IF_NOT_NONE" {
        return Some(Header {
            copies,
            operand,
            compare: at,
            branch: at,
        });
    }
    if !matches!(
        ops.get(at)?.name,
        "LOAD_CONST"
            | "LOAD_SMALL_INT"
            | "LOAD_COMMON_CONSTANT"
            | "LOAD_GLOBAL"
            | "LOAD_NAME"
            | "LOAD_DEREF"
            | "LOAD_FAST"
            | "LOAD_FAST_BORROW"
            | "LOAD_FAST_CHECK"
    ) {
        return None;
    }
    at += 1;
    while ops
        .get(at)
        .is_some_and(|op| op.name == "LOAD_ATTR" && op.arg & 1 == 0)
    {
        at += 1;
    }
    if !matches!(operator(*ops.get(at)?, version), Some("==" | "is")) {
        return None;
    }
    let compare = at;
    at += 1;
    at += usize::from(ops.get(at).is_some_and(|op| op.name == "TO_BOOL"));
    (ops.get(at)?.name == "POP_JUMP_IF_FALSE"
        || (copies == 0 && ops[at].name == "POP_JUMP_IF_TRUE"))
        .then_some(Header {
            copies,
            operand,
            compare,
            branch: at,
        })
}

fn after_test(ops: &[Instruction], branch: usize) -> usize {
    branch + 1 + usize::from(ops.get(branch + 1).is_some_and(|op| op.name == "NOT_TAKEN"))
}

fn store(op: Instruction) -> bool {
    matches!(
        op.name,
        "STORE_NAME" | "STORE_GLOBAL" | "STORE_FAST" | "STORE_DEREF" | "STORE_FAST_LOAD_FAST"
    )
}

struct CaseTest {
    pattern: String,
    copies: usize,
    body: usize,
    fail: usize,
    body_end: usize,
    fallback: Option<std::ops::Range<usize>>,
}

impl Body<'_, '_, '_> {
    fn pattern_operand(
        &mut self,
        ops: &[Instruction],
        test: &Header,
        level: usize,
    ) -> Result<String, String> {
        Ok(if test.compare == test.branch {
            "None".into()
        } else {
            let text = self
                .fragment(ops, test.operand, test.compare, level)?
                .expression()?;
            let parsed = ruff_python_parser::parse_expression(&text).map_err(|e| e.to_string())?;
            Generator::new(&Indentation::default(), LineEnding::Lf).expr(parsed.expr())
        })
    }

    fn copied_alternatives(
        &mut self,
        ops: &[Instruction],
        first: std::ops::Range<usize>,
        copies: usize,
        first_pattern: &str,
        level: usize,
    ) -> Result<Option<CaseTest>, String> {
        let suite = &ops[first.clone()];
        if copies == 0
            || suite[0].name != "POP_TOP"
            || !matches!(suite.last().unwrap().name, "RETURN_VALUE" | "RETURN_CONST")
        {
            return Ok(None);
        }
        let mut last = first.clone();
        let mut pattern = first_pattern.to_owned();
        while let Some(next) = header(ops, last.end, self.m.version)
            && next.copies == 1
        {
            let fall = after_test(ops, next.branch);
            let fail = self.flow.target(next.branch)?;
            let Some(candidate) = ops.get(fall..fail) else {
                break;
            };
            self.charge(candidate.len())?;
            if candidate.len() != suite.len()
                || !candidate
                    .iter()
                    .zip(suite)
                    .all(|(a, b)| a.opcode == b.opcode && a.arg == b.arg && a.jump == b.jump)
            {
                break;
            }
            let rhs = self.pattern_operand(ops, &next, level)?;
            write!(pattern, " | {rhs}").unwrap();
            last = fall..fail;
        }
        if last == first || ops.get(last.end).is_none_or(|op| op.name != "POP_TOP") {
            return Ok(None);
        }
        Ok(Some(CaseTest {
            copies: copies - 1,
            pattern,
            body: last.start + 1,
            body_end: last.end,
            fail: last.end + 1,
            fallback: None,
        }))
    }

    fn pattern_test(
        &mut self,
        ops: &[Instruction],
        mut test: Header,
        level: usize,
    ) -> Result<CaseTest, String> {
        let mut copies = test.copies;
        let mut pattern = self.pattern_operand(ops, &test, level)?;
        if ops[test.branch].name == "POP_JUMP_IF_TRUE" {
            // At the end of a loop, a failed final case can fall through to
            // a copy of the latch. Its successful comparison jumps over it.
            let latch = after_test(ops, test.branch);
            let body = self.flow.target(test.branch)?;
            if copies != 0
                || body != latch + 1
                || body >= ops.len()
                || self.loop_transfer(ops, latch) != Some("continue")
                || !self.at_loop_tail(ops.len())
            {
                return Err("unsupported reversed value pattern".into());
            }
            return Ok(CaseTest {
                pattern,
                copies,
                body,
                body_end: ops.len(),
                fail: ops.len(),
                fallback: None,
            });
        }
        let mut matched = None;
        loop {
            let fail = self.flow.target(test.branch)?;
            let fall = after_test(ops, test.branch);
            if fail <= fall || fail > ops.len() {
                return Err("invalid value pattern failure".into());
            }
            if ops[fall].name != "JUMP_FORWARD" || copies == 0 {
                if matched.is_some() {
                    return Err("unsupported pattern alternative".into());
                }
                if let Some(test) =
                    self.copied_alternatives(ops, fall..fail, copies, &pattern, level)?
                {
                    return Ok(test);
                }
                return Ok(CaseTest {
                    pattern,
                    copies,
                    body: fall,
                    body_end: fail,
                    fail,
                    fallback: None,
                });
            }
            // All successful alternatives join at the same subject discard.
            let success = self.flow.target(fall)?;
            if success <= fail
                || success >= ops.len()
                || ops[success].name != "POP_TOP"
                || matched.is_some_and(|dest| dest != success)
            {
                return Err("inconsistent pattern alternatives".into());
            }
            matched = Some(success);
            if let Some(next) = header(ops, fail, self.m.version)
                && next.copies == 1
            {
                let rhs = self.pattern_operand(ops, &next, level)?;
                write!(pattern, " | {rhs}").unwrap();
                test = next;
                continue;
            }
            let cleanup = fail + 1;
            if ops[fail].name != "POP_TOP" || cleanup >= success {
                return Err("missing alternative failure cleanup".into());
            }
            copies -= 1;
            let (fail, fallback) = if ops[cleanup].name == "JUMP_FORWARD" {
                (self.flow.target(cleanup)?, None)
            } else {
                // A terminal wildcard arm can precede the shared success block.
                (ops.len(), Some(cleanup..success))
            };
            if fail <= success || fail > ops.len() {
                return Err("invalid alternative exit".into());
            }
            return Ok(CaseTest {
                pattern,
                copies,
                body: success + 1,
                body_end: fail,
                fail,
                fallback,
            });
        }
    }

    pub(super) fn value_cases(
        &mut self,
        ops: &[Instruction],
        first: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        let ops = &ops[..end];
        let subject = self.expr()?;
        let mut cases = String::new();
        let mut at = first;
        let mut join = None;
        let mut saved = true;
        while saved && let Some(test) = header(ops, at, self.m.version) {
            let reversed = ops[test.branch].name == "POP_JUMP_IF_TRUE";
            let CaseTest {
                mut pattern,
                copies,
                mut body,
                body_end,
                fail,
                fallback,
            } = self.pattern_test(ops, test, level)?;
            if body >= fail {
                return Err("missing match body".into());
            }
            let mut seed = Vec::new();
            let capture = copies > 0
                && store(ops[body])
                && ops.get(fail).is_some_and(|op| op.name == "POP_TOP");
            if capture {
                let binding = self.binding(ops, body, fail)?;
                write!(pattern, " as {}", binding.target).unwrap();
                body = binding.next;
                seed = binding.seed;
            }
            saved = copies > usize::from(capture);
            let next = fail + usize::from(capture);
            if !reversed && body < fail {
                let guard = self.flow.block(body).ops.end - 1;
                if guard < fail
                    && ops[guard].name == "POP_JUMP_IF_FALSE"
                    && self.flow.jump(guard) == Some(next)
                    && join != Some(next)
                {
                    let stop =
                        guard - usize::from(guard > body && ops[guard - 1].name == "TO_BOOL");
                    let condition = self
                        .seeded(ops, body, stop, level, std::mem::take(&mut seed))?
                        .expression()?;
                    write!(pattern, " if {condition}").unwrap();
                    body = guard
                        + 1
                        + usize::from(ops.get(guard + 1).is_some_and(|op| op.name == "NOT_TAKEN"));
                }
            }
            if saved {
                if ops.get(body).is_none_or(|op| op.name != "POP_TOP") {
                    return Err("missing saved match subject".into());
                }
                body += 1;
            }
            let tail = body_end - 1;
            let stop = if ops[tail].name == "JUMP_FORWARD" {
                let after = self.flow.target(tail)?;
                if after < next || after > end || join.is_some_and(|previous| previous != after) {
                    return Err("inconsistent match continuation".into());
                }
                join = Some(after);
                tail
            } else if body_end < end
                && self.at_loop_tail(end)
                && self.loop_transfer(ops, tail) == Some("continue")
            {
                tail
            } else {
                body_end
            };
            // After a successful pattern has discarded its subject, a test
            // can skip the remaining cases and go straight to the shared
            // continuation. A failed match guard would try the next case.
            let suite = if let Some(branch) =
                (body < stop).then(|| self.flow.block(body).ops.end - 1)
                && branch < stop
                && ops[branch].name == "POP_JUMP_IF_FALSE"
                && matches!(
                    ops[tail].name,
                    "RETURN_VALUE" | "RETURN_CONST" | "RAISE_VARARGS"
                )
                && let Some(after) = self.flow.jump(branch)
                && after > next
                && after <= end
            {
                if join.is_some_and(|previous| previous != after) {
                    return Err("inconsistent match continuation".into());
                }
                join = Some(after);
                let mut prefix = self.seeded(ops, body, branch, level, seed)?;
                let test = prefix
                    .stack
                    .pop()
                    .and_then(Value::expression_text)
                    .ok_or("missing case condition")?;
                let mut suite = prefix.suite()?;
                let yes = self
                    .fragment(ops, after_test(ops, branch), stop, level)?
                    .suite()?;
                write_suite(&mut suite, format_args!("if {test}"), &yes);
                suite
            } else {
                self.seeded(ops, body, stop, level, seed)?.suite()?
            };
            case(&mut cases, &pattern, &suite);
            if let Some(range) = fallback {
                let suite = self.fragment(ops, range.start, range.end, level)?.suite()?;
                if !suite.is_empty() {
                    case(&mut cases, "_", &suite);
                }
            }
            at = next;
        }
        let after = join.unwrap_or(end);
        if at > after {
            return Err("match overlaps its continuation".into());
        }
        if at < after {
            let mut pattern = "_".to_owned();
            let mut seed = Vec::new();
            if saved {
                if !store(ops[at]) {
                    return Err("unsupported following match pattern".into());
                }
                let binding = self.binding(ops, at, after)?;
                pattern = binding.target;
                seed = binding.seed;
                at = binding.next;
            }
            let fallback = self.seeded(ops, at, after, level, seed)?.suite()?;
            if !fallback.is_empty() || saved {
                if fallback.lines().count() == 1 && ops.get(at).is_some_and(|op| op.name != "NOP") {
                    writeln!(cases, "case {pattern}: {}", fallback.trim_end()).unwrap();
                } else {
                    case(&mut cases, &pattern, &fallback);
                }
            }
        }
        self.block(&format!("match {subject}:"), &cases)?;
        Ok(after)
    }
}
