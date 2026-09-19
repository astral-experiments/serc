use crate::common;

#[test]
fn matches_cpython_marshal_for_fold_admissions_after_constant_assertions() {
    for (first, second) in [
        ("-2", "-1"),
        ("257 + 2", "257 + 3"),
        ("~1", "~2"),
        ("-1.5", "-2.5"),
        ("1.5 + 2.5", "1.5 + 3.5"),
        ("'al' + 'pha'", "'be' + 'ta'"),
    ] {
        for prefix in [String::new(), format!("    work({second})\n")] {
            for following in ["", "    work('between')\n"] {
                let source = format!(
                    "def f(value, flag):\n{prefix}    assert value.kind == flag.kind == other.kind\n    if flag is not None:\n        assert False, 'stop'\n        work({first})\n        work({second})\n{following}    work({second})\n    work({first})\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "discarded_fold_admissions.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_pruned_folded_constants() {
    for value in [
        "((-0.0, 1) if other else (0.0, 1)), 2",
        "((-3, 1) if other else (-4, 1)), 2",
        "(('a' + 'b', 1) if other else ('c' + 'd', 1)), 2",
        "((258 - 3, 1) if other else (259 - 3, 1)), 2",
    ] {
        for terminal in ["assert False, 'stop'", "return None", "raise ValueError"] {
            for following in ["", "    work((-9, 2))\n    work((-8, 2))\n"] {
                let source = format!(
                    "def f(value, flag, other):\n    assert value.a == value.b == value.c\n    if flag:\n        {terminal}\n        return {value}\n{following}    return True\n"
                );
                common::assert_matches_cpython_marshal(&source, "pruned_folded_constants.py");
            }
        }
    }

    common::assert_matches_cpython_marshal(
        "def f(value, flag, other):\n    assert value.a == value.b == value.c\n    if flag:\n        assert False, 'stop'\n        work(-901)\n    if other:\n        return (1 if flag else 2), 3\n    return (4 if flag else 5), 6\n",
        "pruned_constant_alias.py",
    );
}

#[test]
fn preserves_discarded_suite_fold_order_without_cleanup() {
    common::assert_matches_cpython_marshal(
        "if False:\n    sink((5, 3), -1)\nfor item in ['left', 'right']:\n    sink(item)\nsink((5, 3), -0.5, -1)\n",
        "discarded_suite_fold_order.py",
    );
}

#[test]
fn preserves_discarded_suite_fold_order_after_finally_copying() {
    common::assert_matches_cpython_marshal(
        "def sample(stop):\n    if False:\n        sink((5, 3), -1)\n    try:\n        if stop:\n            return\n    finally:\n        sink((7, 11))\n    for item in ['left', 'right']:\n        sink(item)\n    sink((5, 3), -0.5, -1)\n",
        "discarded_suite_finally_fold_order.py",
    );
}

#[test]
fn preserves_discarded_suite_fold_order_across_manager_cleanup() {
    common::assert_matches_cpython_marshal(
        "for token in tokens:\n    with manager():\n        if False:\n            sink((5, 3), -1)\nwith manager():\n    for item in ['left', 'right']:\n        sink(item)\nsink((5, 3), -0.5, -1)\n",
        "discarded_suite_manager_fold_order.py",
    );
}
