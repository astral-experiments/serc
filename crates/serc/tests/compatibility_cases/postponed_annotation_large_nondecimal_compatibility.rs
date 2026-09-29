use crate::common;

#[test]
fn matches_cpython_for_many_postponed_large_nondecimal_integers() {
    let annotation = std::iter::repeat_n("0x1_0000_0000_0000_0000", 10_000)
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("from __future__ import annotations\nvalue: ({annotation})\n");
    common::assert_matches_cpython_marshal(&source, "many_large_nondecimal_integers.py");
}
