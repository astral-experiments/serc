//! Recover independent scopes after an unsupported construct interrupts a module.
use std::ops::Range;

use ruff_text_size::Ranged;

use super::{
    Body, Bytecode, Candidate, MAX_WORK, Module, Object, Recovery, detached, scope, synthetic,
};

/// Names in diagnostics originate in the input and must fit on one comment line.
pub(crate) fn comment(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars().take(480) {
        if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
            result.extend(c.escape_default());
        } else {
            result.push(c);
        }
    }
    if text.chars().nth(480).is_some() {
        result.push('…');
    }
    result
}

pub(crate) fn partial(
    m: &Module<'_>,
    candidate: Option<Candidate>,
    compile: impl FnMut(&str, serc_bytecode::OptimizationLevel) -> Option<Vec<u8>>,
) -> Result<String, String> {
    // A malformed graph is still an input error. Validate even scopes the source
    // pass may skip when their parent contains an unsupported statement.
    if m.text(m.code(m.root)?.name)? != "<module>" {
        return Err("expected module code object".into());
    }
    let mut budget = MAX_WORK;
    let mut implicit = Vec::new();
    for (id, obj) in m.objects.iter().enumerate() {
        if let Object::Code(code) = obj {
            let bytes = m.bytes(code.bytecode)?;
            let handlers = m.bytes(code.exceptions)?;
            budget = budget
                .checked_sub(bytes.len() + handlers.len() + 1)
                .ok_or("bytecode validation work limit exceeded")?;
            let bc = Bytecode::decode(m.version, bytes, handlers)?;
            crate::flow::Graph::build(&bc, m.version, code.stack_size)?;
            if m.text(code.name)? == "__annotate__" && detached::annotation_helper(&bc) {
                implicit.push(id);
            }
        }
    }
    let mut work = Recovery {
        budget: MAX_WORK,
        preserve_nops: false,
        saw_nop: false,
        synthetic: None,
        mode: synthetic::Mode::Ordinary,
        emitted: false,
        partial: true,
        definitions: Vec::new(),
        private_class: None,
    };
    let mut candidate = if let Some(candidate) = candidate {
        candidate
    } else {
        let source = scope(m, m.root, 0, false, &mut work)?;
        ruff_python_parser::parse_module(&source)
            .map_err(|e| format!("invalid partial source: {e}"))?;
        detached::take_marks(source)
    };
    work.definitions.extend(implicit);
    let warned = super::warnings::annotate(m, &mut candidate, compile);
    detached::append(m, candidate, &mut work, &warned)
}

impl Body<'_, '_, '_> {
    pub(super) fn partial_scope(&mut self, reason: &str, bc: &Bytecode) -> Result<String, String> {
        self.charge(self.lines.len())?;
        let prefix = complete_prefix(&self.lines);
        if prefix < self.lines.len() {
            self.lines.truncate(prefix);
            // A syntactically incomplete suite may have been discarded before
            // the last checkpoint. Include its whole scope for context.
            self.partial_start = 0;
        }
        self.gap(reason, &bc.ops, self.partial_start..bc.ops.len(), true)?;
        self.lexical_bindings(&bc.ops)?;
        Ok(std::mem::take(&mut self.lines))
    }

    /// Resume at an empty stack in the same straight-line block, or where both
    /// forward arms rejoin with an empty stack. Never move a branch arm into
    /// the surrounding suite just because it happens to be next in the file.
    pub(super) fn resume_after(
        &mut self,
        ops: &[super::Instruction],
        i: usize,
    ) -> Result<Option<usize>, String> {
        let range = self.flow.block(i).ops.clone();
        let Some(mut height) = self.flow.block(i).height else {
            return Ok(None);
        };
        self.charge(range.len())?;
        for (index, op) in ops[range.clone()].iter().enumerate() {
            let (pop, push) =
                serc_bytecode::target::stack_counts_for(self.m.version, op.opcode, op.arg);
            let Some(after) = height.checked_sub(pop).and_then(|n| n.checked_add(push)) else {
                return Ok(None);
            };
            height = after;
            let next = range.start + index + 1;
            if next > i && next < range.end && height == 0 {
                return Ok(Some(next));
            }
        }
        Ok(self
            .flow
            .join(i)
            .filter(|&n| n > i && self.flow.block(n).height == Some(0)))
    }

    pub(super) fn gap(
        &mut self,
        reason: &str,
        ops: &[super::Instruction],
        range: Range<usize>,
        remainder: bool,
    ) -> Result<(), String> {
        self.remember_cell_stores(&ops[range.clone()])?;
        let name = comment(self.m.text(self.code.qualname)?);
        let extent = if remainder {
            "remainder of"
        } else {
            "statement in"
        };
        self.line(&format!(
            "...  {}{} {} {} # serc: unrecovered {extent} {name}: {}",
            super::blocks::MARK,
            self.id,
            range.start,
            range.end,
            comment(reason)
        ))
    }

    fn remember_cell_stores(&mut self, ops: &[super::Instruction]) -> Result<(), String> {
        self.charge(ops.len())?;
        self.omitted_cell_stores.extend(
            ops.iter()
                .filter(|op| matches!(op.name, "STORE_DEREF" | "DELETE_DEREF"))
                .map(|op| op.offset),
        );
        Ok(())
    }
}

/// A failed handler can leave a dangling `try` after complete definitions.
/// Keep the original slices, including comments marking any synthetic suites.
fn complete_prefix(source: &str) -> usize {
    let parsed =
        ruff_python_parser::parse_unchecked_source(source, ruff_python_ast::PySourceType::Python);
    let Some(error) = parsed.errors().first() else {
        return source.len();
    };
    let statements = parsed.suite();
    let mut end = 0;
    for (i, stmt) in statements.iter().enumerate() {
        let next = statements
            .get(i + 1)
            .map_or(source.len(), |s| s.start().to_usize());
        // The parser can recover multiple statements on one malformed line.
        // A diagnostic appended at such a boundary would join its first token
        // to the retained expression.
        if !source[..next].ends_with('\n')
            || stmt.end() > error.location.start()
            || ruff_python_parser::parse_module(&source[end..next]).is_err()
        {
            break;
        }
        end = next;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::complete_prefix;

    #[test]
    fn partial_prefix_discards_the_entire_malformed_line() {
        let source = "before = 1\nf\"x\"broken\nafter = 2\n";
        assert_eq!(&source[..complete_prefix(source)], "before = 1\n");
    }
}
