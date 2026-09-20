use std::fmt::Write as _;

use crate::common::assert_matches_cpython_marshal;

#[test]
fn frozenset_tuple_reference_order_matches_cpython() {
    let mut dense_prefix = String::new();
    for index in 0..32 {
        writeln!(dense_prefix, "anchor_{index} = {}", 300 + index).unwrap();
    }
    for members in [
        "{('name', 'other'), ('name', 'other-name')}",
        "{(300, 300), (300, 301)}",
        "{((300, 300), 'name'), ((300, 301), 'other-name')}",
        "{((300,), (300,)), ((300,), (301,))}",
        "{(b'long bytes', b'long bytes'), (b'long bytes', b'other bytes')}",
        "{(1.5, 1.5), (1.5, 2.5)}",
        "{(1, 1), (1, 2)}",
        "{('name', 'name'), ('name', 'other'), ('name', 'other-name')}",
    ] {
        for prefix in [
            "",
            "anchor = 301\n",
            "anchor = 'other-name'\n",
            &dense_prefix,
        ] {
            let source = format!("{prefix}result = value in {members}\n");
            assert_matches_cpython_marshal(&source, "frozenset_tuple_reference_order.py");
            let source = format!("{prefix}def f(value):\n    return value in {members}\n");
            assert_matches_cpython_marshal(&source, "frozenset_tuple_reference_order.py");
        }
    }
}
