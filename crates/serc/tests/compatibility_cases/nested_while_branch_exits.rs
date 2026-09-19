use crate::common;

#[test]
fn preserves_non_final_compound_while_branch_layout() {
    for condition in [
        "left or right",
        "left and right",
        "left and (right or other)",
        "not left or right",
        "left if other else right",
    ] {
        for loop_condition in ["flag", "True"] {
            for first_arm in ["", "            if selected:\n                interim()\n"] {
                let branch = if first_arm.is_empty() { "if" } else { "elif" };
                let source = format!(
                    "def function(flag, other, selected, left, right):\n    while {loop_condition}:\n        if other:\n{first_arm}            {branch} {condition}:\n                work()\n        else:\n            fallback()\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_while_branch.py");
            }
        }
    }
}

#[test]
fn preserves_non_final_suspended_while_branch_exits() {
    for (definition, prefix) in [
        ("async def", "    await ready()\n"),
        ("def", "    yield item\n"),
        ("async def", "    await ready()\n    yield item\n"),
    ] {
        for condition in ["left and right", "all(value for value in values)"] {
            let source = format!(
                "{definition} function(values, flag, other, left, right, item):\n{prefix}    while flag:\n        if other:\n            if {condition}:\n                work()\n        else:\n            fallback()\n"
            );
            common::assert_matches_cpython_marshal(&source, "suspended_while_branch.py");
        }
    }
}

#[test]
fn preserves_nested_branch_exits_in_protected_loops() {
    for header in ["for item in values", "while flag"] {
        let source = format!(
            "def function(values, flag, other, left, right):\n    try:\n        {header}:\n            if other:\n                if left and right:\n                    work()\n            else:\n                fallback()\n    except Error:\n        recover()\n"
        );
        common::assert_matches_cpython_marshal(&source, "protected_loop_branch.py");
    }
}
