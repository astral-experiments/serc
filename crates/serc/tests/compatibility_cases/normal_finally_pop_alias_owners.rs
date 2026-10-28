use crate::common::assert_matches_cpython_marshal;

#[test]
fn normal_finally_loop_attribute_none_reuses_owned_original_slot() {
    for (definition, manager, work, comparison) in [
        ("async def", "async with", "await work()", "is not"),
        ("async def", "async with", "await work()", "is"),
        ("async def", "async with", "yield await work()", "is not"),
        ("async def", "with", "await work()", "is not"),
        ("def", "with", "yield work()", "is not"),
    ] {
        let source = format!(
            "{definition} f(manager, state, active, work, cleanup):\n    {manager} manager:\n        try:\n            while active:\n                {work}\n        finally:\n            if state.resource {comparison} None:\n                cleanup()\n"
        );
        assert_matches_cpython_marshal(&source, "normal_finally_loop_attribute_none.py");
    }
}

#[test]
fn normal_finally_attribute_none_preserves_distinct_entry_boundaries() {
    for body in [
        "while active and state.active:\n    await work()",
        "while active:\n    await work()\nwork()",
        "await work()",
    ] {
        let body = body.replace('\n', "\n            ");
        let source = format!(
            "async def f(manager, state, active, work, cleanup):\n    async with manager:\n        try:\n            {body}\n        finally:\n            if state.resource is not None:\n                cleanup()\n"
        );
        assert_matches_cpython_marshal(&source, "normal_finally_attribute_none_boundaries.py");
    }
}

#[test]
fn normal_finally_subscript_markers_follow_the_surviving_pop() {
    for handler in ["break", "raise", "pass", "if ready: break"] {
        for test in ["state.items[key]", "state[key]"] {
            let source = format!(
                "async def f(manager, ready, state, key):\n    async with manager:\n        while ready:\n            try:\n                read()\n            except Error:\n                {handler}\n            finally:\n                if not {test}:\n                    work()\n"
            );
            assert_matches_cpython_marshal(&source, "normal_finally_subscript_marker.py");
        }
    }
}
