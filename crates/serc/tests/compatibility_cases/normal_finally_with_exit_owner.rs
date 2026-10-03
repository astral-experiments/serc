use crate::common;
use crate::common::indent;

#[test]
fn guarded_async_finally_manager_keeps_enclosing_sync_manager_owner() {
    for condition in ["value is not None", "value", "value == enabled"] {
        for recovery in ["", "        except Error:\n            pass\n"] {
            for binding in ["", " as session"] {
                for body in ["pass", "await work()"] {
                    let source = format!(
                        "async def f(manager, value, enabled):\n    with manager:\n        try:\n            work()\n{recovery}        finally:\n            if {condition}:\n                async with session_factory(){binding}:\n                    {body}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "guarded_async_finally_manager.py",
                    );
                }
            }
        }
    }
}

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

#[test]
fn preserves_outer_manager_owner_on_conditional_finally_manager_joins() {
    for body in ["work()", "yield value"] {
        for leading in [
            "",
            "            cleanup()\n            flush()\n            seek(0)\n",
        ] {
            for condition in ["flag", "flag is not None"] {
                let source = format!(
                    "def f(outer, inner, flag, value):\n    with outer():\n        try:\n            {body}\n        finally:\n{leading}            if {condition}:\n                with inner() as resource:\n                    result = resource.read()\n                    work(result)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "conditional_finally_manager_join.py",
                );
            }
        }
    }
}

#[test]
fn preserves_outer_manager_owner_after_sequential_finally_guards() {
    for leading in [
        "            cleanup()\n",
        "            if ready:\n                cleanup()\n",
        "            if ready:\n                cleanup()\n                flush()\n",
    ] {
        let source = format!(
            "def f(outer, inner, ready, resource):\n    with outer():\n        try:\n            work()\n        finally:\n{leading}            if resource is not None:\n                with inner():\n                    resource.flush()\n"
        );
        common::assert_matches_cpython_marshal(&source, "sequential_finally_manager_joins.py");
    }
}
