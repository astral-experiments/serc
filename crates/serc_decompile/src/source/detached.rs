//! Keep named functions whose definition site disappeared from partial source.
use std::fmt::Write;

use ruff_python_ast::{PySourceType, token::TokenKind};
use ruff_text_size::Ranged;
use rustc_hash::FxHashSet;
use serc_bytecode::target::code_flags;

use super::{
    Bytecode, Candidate, CommonConstant, Definition, Id, MAX_SOURCE, Module, Object, Recovery,
    blocks, comment, definitions::function_prefix, identifier, scope,
};

const MARK: &str = "# __serc_definition_id__ ";
const LIMIT: &str = "\n# serc: detached function recovery limit reached\n";

/// CPython's annotation evaluator raises the intrinsic `NotImplementedError`
/// for unsupported formats. An authored function with this name loads the
/// exception class through its ordinary name lookup.
pub(super) fn annotation_helper(bc: &Bytecode) -> bool {
    let Some(guard) = bc.ops.iter().position(|op| op.name == "POP_JUMP_IF_FALSE") else {
        return false;
    };
    let raise = guard
        + 1
        + usize::from(
            bc.ops
                .get(guard + 1)
                .is_some_and(|op| op.name == "NOT_TAKEN"),
        );
    matches!(bc.ops.get(raise..), Some([error, op, ..])
        if (error.name, error.arg) == ("LOAD_COMMON_CONSTANT", CommonConstant::NOTIMPLEMENTEDERROR.argument())
        && (op.name, op.arg) == ("RAISE_VARARGS", 1))
}

// A partial statement can be discarded after its nested definitions have
// been rendered. Carry identities on their headers until the source settles.
pub(super) fn mark(header: &mut String, id: Id) {
    write!(header, "  {MARK}{id}").unwrap();
}

pub(super) fn take_marks(source: String) -> Candidate {
    let source = super::layout::readable_primaries(source);
    if !source.contains(MARK) && !source.contains(blocks::MARK) {
        return Candidate::new(source, Vec::new());
    }
    let mut definitions = Vec::new();
    let mut unplaced = Vec::new();
    let parsed = ruff_python_parser::parse_unchecked_source(&source, PySourceType::Python);
    let mut clean = String::with_capacity(source.len());
    let mut start = 0;
    for token in parsed
        .tokens()
        .iter()
        .filter(|token| token.kind() == TokenKind::Comment)
    {
        if let Some(id) = source[token.range()]
            .strip_prefix(MARK)
            .and_then(|s| s.parse::<Id>().ok())
        {
            clean.push_str(source[start..token.start().to_usize()].trim_end_matches(' '));
            start = token.end().to_usize();
            definitions.push(id);
        } else if let Some((gap, public)) = blocks::read_mark(&source[token.range()]) {
            clean.push_str(&source[start..token.start().to_usize()]);
            clean.push_str(public);
            start = token.end().to_usize();
            unplaced.push(gap);
        }
    }
    clean.push_str(&source[start..]);
    Candidate {
        source: clean,
        definitions,
        unplaced,
    }
}

pub(super) fn append<'m>(
    m: &'m Module<'_>,
    candidate: Candidate,
    work: &mut Recovery<'m>,
    warned: &super::warnings::Warnings,
    formatting_bytes: usize,
) -> Result<String, String> {
    let Candidate {
        mut source,
        definitions,
        mut unplaced,
    } = candidate;
    // Padding and diagnostics must not displace detached bodies.
    let limit =
        MAX_SOURCE + formatting_bytes + warned.values().map(|w| w.annotation_len()).sum::<usize>();
    let mut placed: FxHashSet<_> = definitions.into_iter().collect();
    // Compiler helpers need no detached function, but their instructions still
    // belong in the listing if the expression using them was discarded.
    let mut helpers: FxHashSet<_> = work.definitions.drain(..).collect();
    let mut visited = FxHashSet::default();
    let mut pending = vec![m.root];
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Ok(code) = m.code(id) else {
            continue;
        };
        let Ok(constants) = m.tuple(code.constants) else {
            continue;
        };
        let Some(budget) = work.budget.checked_sub(constants.len() + 1) else {
            if source.len() + LIMIT.len() <= limit {
                source.push_str(LIMIT);
            }
            break;
        };
        work.budget = budget;
        // Visit parents first. A detached body can contain further intact
        // definitions, which then need no separate entry.
        pending.extend(
            constants
                .iter()
                .rev()
                .copied()
                .filter(|&child| matches!(m.objects[child], Object::Code(_))),
        );
        if code.flags & code_flags::CO_OPTIMIZED == 0
            || placed.contains(&id)
            || helpers.contains(&id)
        {
            continue;
        }
        let Ok(name) = m.text(code.name).and_then(identifier) else {
            continue;
        };
        let Ok(qualified) = m.text(code.qualname) else {
            continue;
        };
        let Ok(parameters) = Definition::new(id).parameters(m, code) else {
            continue;
        };
        let Ok(body) = scope(m, id, 1, false, work) else {
            continue;
        };
        let body = take_marks(body);
        let title = format!(
            "\n# serc: detached function {} (original line {})\n# {}def {name}({parameters}):\n",
            comment(qualified),
            code.first_line,
            function_prefix(code)
        );
        // Python also treats a lone carriage return as a physical newline.
        // Prefix every physical line, even when the input contains one.
        let lines = body.source.split_terminator(['\r', '\n']);
        let size = title.len() + body.source.len() + lines.clone().count() * 7;
        if source.len().saturating_add(size) + LIMIT.len() > limit {
            if source.len() + LIMIT.len() <= limit {
                source.push_str(LIMIT);
            }
            break;
        }
        source.push_str(&title);
        for line in lines {
            writeln!(source, "#     {line}").unwrap();
        }
        placed.insert(id);
        unplaced.extend(body.unplaced);
        placed.extend(body.definitions);
        helpers.extend(work.definitions.drain(..));
    }
    blocks::append(m, &mut source, unplaced, &placed, warned)?;
    Ok(source)
}
