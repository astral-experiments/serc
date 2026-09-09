use crate::common;

#[test]
fn matches_cpython_short_circuit_nested_while_backedges() {
    for action in ["consume(value)", "yield value"] {
        for condition in [
            "left",
            "left and right",
            "left or right",
            "left or (middle and right)",
            "left and middle and right",
            "not (left or right)",
        ] {
            let source = format!(
                "def f(values, left, middle, right):\n    for value in values:\n        while {condition}:\n            {action}\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "short_circuit_nested_while_backedges.py",
            );
        }
    }
}

#[test]
fn matches_cpython_short_circuit_final_else_backedges() {
    for action in ["consume(value)", "yield value"] {
        for condition in [
            "left",
            "left and right",
            "left or right",
            "left or (middle and right)",
            "left and middle and right",
            "not (left or right)",
        ] {
            let source = format!(
                "def f(values, outer, first, left, middle, right):\n    for value in values:\n        if outer:\n            consume(value)\n        else:\n            if first:\n                consume(value)\n            elif {condition}:\n                {action}\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "short_circuit_final_else_backedges.py",
            );
        }
    }
}
