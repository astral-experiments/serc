use std::fmt::Write as _;

use super::{NameId, ScopeArena, ScopePlan};

fn names(arena: &ScopeArena, ids: impl IntoIterator<Item = NameId>) -> Vec<&str> {
    ids.into_iter().map(|id| arena.names.resolve(id)).collect()
}

fn sorted_names(arena: &ScopeArena, ids: impl IntoIterator<Item = NameId>) -> Vec<&str> {
    let mut names = names(arena, ids);
    names.sort_unstable();
    names
}

#[test]
fn nested_scope_plan() {
    let source = "global external\ndef outer(value):\n    captured = value\n    class C:\n        field: value\n        def method(self):\n            return captured, __class__\n    return C\n";
    let parsed = ruff_python_parser::parse_module(source).unwrap();
    let arena = ScopeArena::analyze(parsed.suite());
    let mut snapshot = String::new();
    writeln!(
        snapshot,
        "module globals: {:?}",
        names(
            &arena,
            (0..arena.names.len())
                .map(NameId::from_usize)
                .filter(|&id| arena.module.module_globals.contains(id))
        )
    )
    .unwrap();
    for (index, node) in arena.nodes.iter().enumerate() {
        writeln!(snapshot, "scope {index} children={:?}", node.children).unwrap();
        match &node.plan {
            ScopePlan::Module => writeln!(snapshot, "  module").unwrap(),
            ScopePlan::Function(plan) => {
                writeln!(
                    snapshot,
                    "  function locals={:?}",
                    names(&arena, plan.locals.iter().copied())
                )
                .unwrap();
                writeln!(
                    snapshot,
                    "  cells={:?} free={:?} globals={:?}",
                    sorted_names(&arena, plan.cellvars.iter().copied()),
                    names(&arena, plan.freevars.iter().copied()),
                    sorted_names(&arena, plan.globals.iter().copied())
                )
                .unwrap();
                writeln!(
                    snapshot,
                    "  annotation references={:?} free={:?}",
                    sorted_names(&arena, plan.annotation_references.iter().copied()),
                    names(&arena, plan.annotation_freevars.iter().copied())
                )
                .unwrap();
            }
            ScopePlan::Class(plan) => {
                writeln!(
                    snapshot,
                    "  class bound={:?} required={:?}",
                    sorted_names(&arena, plan.bound_names.iter().copied()),
                    sorted_names(&arena, plan.required.iter().copied())
                )
                .unwrap();
                writeln!(
                    snapshot,
                    "  classcell={} classdict={}",
                    plan.needs_class_closure, plan.needs_classdict
                )
                .unwrap();
            }
        }
    }
    insta::assert_snapshot!(snapshot);
}
