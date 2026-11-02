//! Recover independent scopes after an unsupported construct interrupts a module.
use std::fmt::Write;

use super::{Body, Bytecode, MAX_WORK, Module, Object, Recovery, scope, synthetic};

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

pub(crate) fn partial(m: &Module<'_>, candidate: Option<String>) -> Result<String, String> {
    // A malformed graph is still an input error. Validate even scopes the source
    // pass may skip when their parent contains an unsupported statement.
    if m.text(m.code(m.root)?.name)? != "<module>" {
        return Err("expected module code object".into());
    }
    let mut budget = MAX_WORK;
    for obj in &m.objects {
        if let Object::Code(code) = obj {
            let bytes = m.bytes(code.bytecode)?;
            let handlers = m.bytes(code.exceptions)?;
            budget = budget
                .checked_sub(bytes.len() + handlers.len() + 1)
                .ok_or("bytecode validation work limit exceeded")?;
            let bc = Bytecode::decode(m.version, bytes, handlers)?;
            crate::flow::Graph::build(&bc, m.version, code.stack_size)?;
        }
    }
    if let Some(source) = candidate {
        return Ok(source);
    }
    let mut work = Recovery {
        budget: MAX_WORK,
        preserve_nops: false,
        saw_nop: false,
        synthetic: None,
        mode: synthetic::Mode::Ordinary,
        emitted: false,
        partial: true,
    };
    let source = scope(m, m.root, 0, false, &mut work)?;
    ruff_python_parser::parse_module(&source)
        .map_err(|e| format!("invalid partial source: {e}"))?;
    Ok(source)
}

impl Body<'_, '_, '_> {
    pub(super) fn partial_scope(
        &mut self,
        name: &str,
        reason: &str,
        bc: &Bytecode,
    ) -> Result<String, String> {
        // A failed compound statement can leave an incomplete try or decorated
        // definition in the buffer. Only retain a syntactically complete prefix.
        if ruff_python_parser::parse_module(&self.lines).is_err() {
            self.lines.clear();
        }
        self.line(&format!(
            "...  # serc: unrecovered remainder of {}: {}",
            comment(name),
            comment(reason)
        ))?;
        // Include the scope's instruction offsets for inspection even when the
        // first statement is unsupported. Keep the fallback bounded.
        let offset = reason
            .rsplit_once("byte ")
            .and_then(|(_, rest)| rest.split_whitespace().next()?.parse::<usize>().ok());
        let start = offset
            .and_then(|at| bc.ops.binary_search_by_key(&at, |op| op.offset).ok())
            .unwrap_or(0);
        let mut excerpt = String::new();
        for op in bc.ops.iter().skip(start).take(12) {
            writeln!(excerpt, "# {:>4}  {} {}", op.offset, op.name, op.arg).unwrap();
        }
        for line in excerpt.lines() {
            self.line(line)?;
        }
        Ok(std::mem::take(&mut self.lines))
    }
}
