//! Attach the original control-flow graph to gaps that survived source recovery.
use std::collections::BTreeMap;
use std::fmt::Write;
use std::ops::Range;

use rustc_hash::FxHashSet;

use super::{Bytecode, Id, Module, Object, comment};
use crate::flow::Graph;

pub(super) const MARK: &str = "# __serc_unplaced__ ";
const MAX_LISTING: usize = 16 << 20;
const TRUNCATED: &str = "# serc: block listing truncated at the output limit\n";

pub(super) struct Gap {
    pub(super) scope: Id,
    ops: Range<usize>,
}

pub(super) fn read_mark(token: &str) -> Option<(Gap, &str)> {
    let mut fields = token.strip_prefix(MARK)?.splitn(4, ' ');
    let scope = fields.next()?.parse().ok()?;
    let start = fields.next()?.parse().ok()?;
    let end = fields.next()?.parse().ok()?;
    let public = fields.next()?;
    public.starts_with("# serc: unrecovered ").then_some((
        Gap {
            scope,
            ops: start..end,
        },
        public,
    ))
}

/// Give block listings their own budget so a large graph cannot displace a
/// recovered definition. Truncation always occurs between physical lines.
struct Listing<'a> {
    source: &'a mut String,
    end: usize,
}

impl Listing<'_> {
    fn line(&mut self, args: std::fmt::Arguments<'_>) -> bool {
        let start = self.source.len();
        writeln!(self.source, "{args}").unwrap();
        if self.source.len() > self.end {
            self.source.truncate(start);
            self.source.push_str(TRUNCATED);
            return false;
        }
        true
    }
}

pub(super) fn append(
    m: &Module<'_>,
    source: &mut String,
    gaps: Vec<Gap>,
    placed: &FxHashSet<Id>,
    warned: &super::warnings::Warnings,
) -> Result<(), String> {
    let mut pending = BTreeMap::<Id, Vec<Range<usize>>>::new();
    for gap in gaps {
        if !gap.ops.is_empty() {
            pending.entry(gap.scope).or_default().push(gap.ops);
        }
    }
    for &id in warned.keys() {
        pending.entry(id).or_default().push(0..usize::MAX);
    }
    let mut seen = FxHashSet::default();
    let end = source.len() + MAX_LISTING;
    let mut listing = Listing { source, end };
    while let Some((id, mut ranges)) = pending.pop_first() {
        if !seen.insert(id) {
            continue;
        }
        let code = m.code(id)?;
        let bc = Bytecode::decode(
            m.version,
            m.bytes(code.bytecode)?,
            m.bytes(code.exceptions)?,
        )?;
        let flow = Graph::build(&bc, m.version, code.stack_size)?;
        let constants = m.tuple(code.constants)?;
        let warning = warned.get(&id);
        let label = warning.map_or_else(
            || "unplaced blocks".to_owned(),
            |w| format!("original blocks ({})", w.description()),
        );
        if !listing.line(format_args!(
            "\n# serc: {label} in {} (original line {}, code {})",
            comment(m.text(code.qualname)?),
            code.first_line,
            id
        )) {
            break;
        }
        ranges.sort_unstable_by_key(|r| r.start);
        let mut covered = 0;
        let mut listed = 0;
        for mut range in ranges {
            range.start = range.start.max(covered);
            range.end = range.end.min(bc.ops.len());
            if range.is_empty() {
                continue;
            }
            // A lost class, lambda, or compiler-generated scope may have no
            // Python definition in either the source or the detached appendix.
            for op in &bc.ops[range.clone()] {
                if warning.is_none()
                    && op.name == "LOAD_CONST"
                    && let Some(&child) = constants.get(op.arg as usize)
                    && matches!(m.objects[child], Object::Code(_))
                    && !placed.contains(&child)
                    && !seen.contains(&child)
                {
                    let child_ranges = pending.entry(child).or_default();
                    if child_ranges.is_empty() {
                        child_ranges.push(0..usize::MAX);
                    }
                }
            }
            covered = range.end;
            range.start = range.start.max(listed);
            for block in flow.blocks_covering(range) {
                let start = block.ops.start;
                let mut header = format!("# block {}", bc.ops[start].offset);
                if let Some(next) = flow.fallthrough(block) {
                    write!(header, "  fallthrough {}", bc.ops[next].offset).unwrap();
                }
                if let Some(jump) = flow.jump(block.ops.end - 1) {
                    write!(header, "  jump {}", bc.ops[jump].offset).unwrap();
                }
                if let Some(h) = flow.handler(start) {
                    write!(
                        header,
                        "  except {} (depth {}, lasti {})",
                        h.target, h.depth, h.lasti
                    )
                    .unwrap();
                }
                if !listing.line(format_args!("{header}")) {
                    return Ok(());
                }
                for op in &bc.ops[block.ops.clone()] {
                    if !listing.line(format_args!("# {:>5}  {} {}", op.offset, op.name, op.arg)) {
                        return Ok(());
                    }
                }
                listed = block.ops.end;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_preserves_source_and_whole_lines() {
        let mut source = "before = 1\n".to_string();
        let end = source.len() + 4;
        let mut listing = Listing {
            source: &mut source,
            end,
        };
        assert!(listing.line(format_args!("# x")));
        assert!(!listing.line(format_args!("# 1234")));
        assert_eq!(source, format!("before = 1\n# x\n{TRUNCATED}"));
    }
}
