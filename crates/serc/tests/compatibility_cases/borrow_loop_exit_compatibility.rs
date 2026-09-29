use crate::common;

#[test]
fn folded_break_condition_keeps_original_loop_exit() {
    for (condition, result) in [
        ("flag is None or True", "left, right, edges"),
        ("flag is None or other", "left, right, edges"),
        ("flag is None or False", "left, right, edges"),
        ("flag is None and True", "left, right, edges"),
        ("True", "left, right, edges"),
        ("flag is None or True", "left"),
    ] {
        let source = format!(
            "def run(flag, other, source):\n    left = []\n    right = []\n    edges = []\n    while True:\n        flag = choose()\n        if {condition}:\n            for item in source:\n                left.append(item)\n            for item in source:\n                right.append(item)\n            break\n        else:\n            for item in source:\n                left.append(item)\n            for item in source:\n                right.append(item)\n            edges.append(flag)\n    return {result}\n"
        );
        common::assert_matches_cpython_marshal(&source, "folded_break_condition.py");
    }
}
