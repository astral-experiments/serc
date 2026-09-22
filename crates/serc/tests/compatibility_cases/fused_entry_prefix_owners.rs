use std::fmt::Write;

use crate::common;

fn assertion_source(calls: usize, arguments: usize) -> String {
    let parameters = (0..18)
        .map(|index| format!("v{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = (0..arguments)
        .map(|index| format!("v{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut source = format!("def f({parameters}):\n");
    for index in 0..calls {
        writeln!(source, "    a{index} = g({arguments})").unwrap();
    }
    source.push_str(
        "    try:\n        assert (v16 == v17).all()\n    except AssertionError:\n        pass\n",
    );
    source
}

#[test]
fn matches_cpython_fused_unprotected_entry_prefix() {
    // Eight one-slot fusions reach SETUP; nine reach the unprotected prefix.
    for (calls, arguments) in [(1, 2), (8, 2), (9, 2), (12, 2), (4, 4), (6, 4)] {
        common::assert_matches_cpython_marshal(
            &assertion_source(calls, arguments),
            "fused_unprotected_entry_prefix.py",
        );
    }
}

#[test]
fn matches_cpython_fused_prefix_with_owner_and_prologue() {
    let plain = assertion_source(9, 2);
    let (header, body) = plain.split_once('\n').unwrap();
    let mut nested = String::new();
    for line in body.lines() {
        writeln!(nested, "    {line}").unwrap();
    }
    let managed = format!("{header}\n    with manager():\n{nested}");
    let generator = format!("{plain}    yield v16\n");
    for source in [managed, generator] {
        common::assert_matches_cpython_marshal(&source, "fused_prefix_with_owner_and_prologue.py");
    }
}
