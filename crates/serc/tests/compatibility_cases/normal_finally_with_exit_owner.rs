use crate::common;

#[test]
fn matches_cpython_marshal_for_normal_finally_after_coroutine_with_return() {
    for manager in ["with", "async with"] {
        for cleanup in ["cleanup(flag=True)", "await cleanup(flag=True)"] {
            for value in ["None", "value.read()"] {
                let source = format!(
                    "async def f(manager, cleanup):\n    try:\n        {manager} manager as value:\n            return {value}\n    finally:\n        {cleanup}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_with_exit_owner.py",
                );
            }
        }
    }
}

#[test]
fn preserves_manager_owners_on_complete_finally_branch_joins() {
    for body in ["work()", "yield None"] {
        for otherwise in ["finish()", "mapping[key] = value"] {
            for manager in [false, true] {
                let cleanup = format!(
                    "try:\n    {body}\nfinally:\n    if flag:\n        cleanup()\n    else:\n        {otherwise}\n"
                );
                let suite = if manager {
                    format!("with manager:\n{}", indent(&cleanup))
                } else {
                    cleanup
                };
                let source = format!(
                    "def f(manager, flag, mapping, key, value):\n{}",
                    indent(&suite)
                );
                common::assert_matches_cpython_marshal(&source, "finally_branch_join_owners.py");
            }
        }
    }
}

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn preserves_finally_exits_with_one_reachable_arm() {
    for body in ["work()", "yield None"] {
        for abrupt in ["return", "raise Error"] {
            for (test, first, otherwise) in [
                ("flag", abrupt, "finish()"),
                ("False", abrupt, "finish()"),
                ("True", "cleanup()", abrupt),
            ] {
                let source = format!(
                    "def f(manager, flag):\n    with manager:\n        try:\n            {body}\n        finally:\n            if {test}:\n                {first}\n            else:\n                {otherwise}\n"
                );
                common::assert_matches_cpython_marshal(&source, "finally_single_arm_exit.py");
            }
        }
    }
}
