use crate::common;

#[test]
fn matches_cpython_for_many_postponed_integer_attributes() {
    let annotation = std::iter::repeat_n("(1).real", 10_000)
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("from __future__ import annotations\nvalue: ({annotation})\n");
    common::assert_matches_cpython_marshal(&source, "many_integer_attributes.py");
}
