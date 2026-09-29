use crate::common;

#[test]
fn preserves_coroutine_owner_after_guarded_empty_recovery() {
    for scope in ["def", "async def"] {
        for guard in ["x", "x or y"] {
            for name in ["", " as inner"] {
                for outer in ["pass", "raise"] {
                    for tail in [
                        "    return use(x)\n",
                        "    work(x)\n    return use(y)\n",
                        "    try:\n        pass\n    finally:\n        try:\n            pass\n        except E:\n            pass\n",
                        "    try:\n        pass\n    finally:\n        cleanup(x)\n",
                    ] {
                        let source = format!(
                            "{scope} f(x, y):\n    try:\n        if {guard}:\n            try:\n                pass\n            except E{name}:\n                pass\n    except E:\n        {outer}\n{tail}"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "coroutine_guarded_empty_recovery_owner.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn empty_try_cleanup_copies_retain_outer_handler_owners() {
    for middle in [" as middle", ""] {
        for inner in [" as inner", ""] {
            for work in ["pass", "work()"] {
                let body = format!(
                    "try:\n    pass\nexcept Error as outer:\n    try:\n        {work}\n    except Error{middle}:\n        try:\n            pass\n        except Error{inner}:\n            pass\n    report(outer)\n"
                );
                for source in [body.clone(), format!("class C:\n{}", common::indent(&body))] {
                    common::assert_matches_cpython_marshal(&source, "empty_try_cleanup_owner.py");
                }
            }
        }
    }
}

#[test]
fn empty_try_nop_retains_enclosing_finally_owner() {
    for handler in ["except:", "except Error:", "except Error as error:"] {
        for action in ["return None", "raise"] {
            for continuation in ["with manager:\n    pass", "work(value, value, value)"] {
                for cleanup in ["pass", "cleanup()"] {
                    let body = format!("try:\n    pass\n{handler}\n    {action}\n{continuation}\n");
                    let source = format!(
                        "def operation():\n    try:\n{}    finally:\n        {cleanup}\n",
                        common::indent(&common::indent(&body))
                    );
                    common::assert_matches_cpython_marshal(&source, "empty_try_finally_owner.py");
                }
            }
        }
    }
}

#[test]
fn empty_try_markers_do_not_add_uninitialized_local_exception_edges() {
    for body in ["pass", "work()"] {
        for source in [
            format!(
                "def function():\n    try:\n        {body}\n    except Error:\n        value = None\n    else:\n        return\n    finally:\n        consume(value)\n"
            ),
            format!(
                "def function():\n    try:\n        first = 1\n    finally:\n        try:\n            {body}\n        finally:\n            try:\n                second = 1\n            finally:\n                consume(second)\n"
            ),
        ] {
            common::assert_matches_cpython_marshal(&source, "empty_try_local_exception_edges.py");
        }
    }
}

#[test]
fn folded_pass_marker_compacts_to_the_ordinary_try_exit() {
    let body = "try:\n    if True:\n        pass\n    else:\n        pass\nexcept FirstError:\n    pass\nexcept SecondError:\n    pass\n";
    for suite in [
        body.to_string(),
        format!("with manager():\n{}", common::indent(body)),
        format!(
            "try:\n{}except OuterError:\n    pass\n",
            common::indent(body)
        ),
    ] {
        for source in [
            format!("{suite}consume(value)\n"),
            format!(
                "def f(value):\n{}    return value\n",
                common::indent(&suite)
            ),
        ] {
            common::assert_matches_cpython_marshal(&source, "folded_pass_try_exit_marker.py");
        }
    }
}

#[test]
fn folded_pass_try_handoff_keeps_nested_managers_and_following_work() {
    for body in [
        "with manager():\n    if True:\n        pass\n    else:\n        pass\n",
        "if True:\n    pass\nelse:\n    pass\nwork()\n",
    ] {
        let suite = format!("try:\n{}except Error:\n    pass\n", common::indent(body));
        for source in [
            format!("{suite}consume(value)\n"),
            format!(
                "def f(value):\n{}    return value\n",
                common::indent(&suite)
            ),
        ] {
            common::assert_matches_cpython_marshal(&source, "folded_pass_try_handoff_boundary.py");
        }
    }
}

#[test]
fn same_line_pass_retains_the_whole_try_setup_span() {
    for prefix in [
        "",
        "try: work()\nexcept Error: pass\n",
        "try: work()\nexcept* Error: pass\n",
    ] {
        let body = format!("{prefix}try: pass\nfinally: pass\nconsume(value)\n");
        let source = format!("def f(value):\n{}", common::indent(&body));
        common::assert_matches_cpython_marshal(&source, "same_line_pass_try_setup_span.py");
    }
}

#[test]
fn same_line_pass_setup_handoff_keeps_neighboring_markers() {
    for body in [
        "try:\n    pass\nfinally: pass\nconsume(value)\n",
        "try: work()\nfinally: pass\nconsume(value)\n",
        "try: pass\nfinally: cleanup()\nconsume(value)\n",
        "with manager():\n    try: pass\n    finally: pass\nconsume(value)\n",
        "try:\n    try: pass\n    finally: pass\nexcept Error: pass\nconsume(value)\n",
        "try: pass\nfinally: pass\n",
        "yield value\ntry: pass\nfinally: pass\nyield value\n",
    ] {
        let source = format!("def f(value):\n{}", common::indent(body));
        common::assert_matches_cpython_marshal(&source, "same_line_pass_setup_boundary.py");
    }
}
