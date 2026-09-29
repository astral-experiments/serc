use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_normal_finally_source_join_owners() {
    for (scope, suspension, close) in [
        ("def", "work()", "close(value)"),
        ("async def", "await work()", "await close(value)"),
        ("def", "yield value", "close(value)"),
        ("async def", "yield value", "await close(value)"),
    ] {
        for manager in ["with", "async with"] {
            if manager == "async with" && scope != "async def" {
                continue;
            }
            for condition in [
                "value is not None",
                "value is None",
                "value.attr",
                "value == enabled",
                "value in choices",
                "value and enabled",
            ] {
                let body = format!(
                    "try:\n    if enabled:\n        {suspension}\n    else:\n        other()\nfinally:\n    if {condition}:\n        {close}\n"
                );
                let body = format!("{manager} manager:\n{}", indent(&body));
                let source = format!(
                    "{scope} f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "normal_finally_source_join.py");
            }
        }
    }
}

#[test]
fn matches_cpython_normal_finally_structural_marker_owners() {
    for suspension in ["await work()", "yield value"] {
        for shape in ["expression", "iterator", "linear"] {
            let body = match shape {
                "expression" => "item = value if enabled else limit".to_string(),
                "iterator" => format!("for item in choices:\n    {suspension}"),
                _ if suspension == "yield value" => "yield value\nwork()\nwork()".to_string(),
                _ => suspension.to_string(),
            };
            for condition in ["value is not None", "value is None"] {
                let body = format!(
                    "try:\n{}finally:\n    if {condition}:\n        await close(value)\n",
                    indent(&body),
                );
                let body = format!("async with manager:\n{}", indent(&body));
                let source = format!(
                    "async def f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_structural_owner.py",
                );
            }
        }
    }
}

#[test]
fn preserves_normal_finally_original_prefix_owners() {
    for (scope, suspension, close) in [
        ("def", "work()", "close(value)"),
        ("async def", "await work()", "await close(value)"),
        ("def", "yield value", "close(value)"),
        ("async def", "yield value", "await close(value)"),
    ] {
        for prefix in ["work()\nwork()\n", "work()\nwork()\nwork()\n"] {
            for condition in ["value is not None", "value is None"] {
                let body = format!("if enabled:\n    {suspension}\nelse:\n    other()\n{prefix}");
                let body = format!(
                    "try:\n{}finally:\n    if {condition}:\n        {close}\n",
                    indent(&body),
                );
                let source = format!(
                    "{scope} f(value, enabled, manager, choices, limit):\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_original_prefix.py",
                );
            }
        }
    }
}

#[test]
fn retained_normal_finally_nop_inherits_through_original_empty_prefix() {
    for (scope, body) in [
        (
            "async def",
            "async for item in stream:\n    await work(item)",
        ),
        ("async def", "async for item in stream:\n    yield item"),
        (
            "async def",
            "async for item in stream:\n    yield item\ncompleted = True",
        ),
        ("def", "for item in stream:\n    work(item)"),
        ("def", "value = work()"),
        ("def", "yield stream\nvalue = work()"),
    ] {
        for cleanup in [
            "try:\n    close()\nexcept Error:\n    pass\n",
            "try:\n    close()\nfinally:\n    release()\n",
        ] {
            let body = format!(
                "try:\n{}except Error:\n    raise\nfinally:\n{}",
                indent(body),
                indent(cleanup),
            );
            let source = format!(
                "{scope} f(stream, work, close, release):\n{}",
                indent(&body),
            );
            common::assert_matches_cpython_marshal(&source, "retained_finally_empty_prefix.py");
        }
    }
}

#[test]
fn shared_and_fallthrough_handler_finally_joins_keep_their_locations() {
    for (body, handler, cleanup) in [
        (
            "if enabled:\n    work()",
            "raise",
            "try:\n    close()\nfinally:\n    release()",
        ),
        (
            "for item in stream:\n    if item:\n        break\n    work()",
            "raise",
            "try:\n    close()\nfinally:\n    release()",
        ),
        (
            "value = work()",
            "recover()",
            "try:\n    close()\nfinally:\n    release()",
        ),
        ("value = work()", "raise", "close()"),
    ] {
        let body = format!(
            "try:\n{}except Error:\n{}finally:\n{}",
            indent(body),
            indent(handler),
            indent(cleanup),
        );
        let source = format!(
            "def f(enabled, stream, work, close, release, recover):\n{}",
            indent(&body),
        );
        common::assert_matches_cpython_marshal(&source, "guarded_finally_empty_prefix.py");
    }
}

#[test]
fn preserves_trailing_pass_after_compound_try_body() {
    for definition in ["def", "async def"] {
        for body in [
            "if flag:\n    work()",
            "with manager():\n    work()",
            "try:\n    work()\nexcept Error:\n    other()",
        ] {
            let source = format!(
                "{definition} f(flag):\n    try:\n        {}\n        pass\n    finally:\n        cleanup()\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "compound_try_body_trailing_pass.py");
        }
    }
}

#[test]
fn normal_finally_comparisons_reuse_their_original_owned_slots() {
    for (scope, suspension) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
        ("async def", "yield await work()"),
    ] {
        for protected in [
            format!("while value:\n    {suspension}\n"),
            format!("try:\n    {suspension}\nexcept Error as error:\n    raise error\n"),
        ] {
            for comparison in ["value > 0", "value == limit", "state.inner.value == ACTIVE"] {
                let body = format!(
                    "try:\n{}finally:\n    if {comparison}:\n        clean()\n",
                    indent(&protected),
                );
                let source = format!(
                    "{scope} f(manager, value, limit, state):\n    with manager:\n{}",
                    indent(&indent(&body)),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_comparison_slot.py",
                );
            }
        }
    }
}

#[test]
fn preserves_pass_finalizer_before_copied_return() {
    for (definition, body) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
    ] {
        for returned in ["None", "value", "42", "work()"] {
            let source = format!(
                "{definition} f(value):\n    try:\n        {body}\n    finally:\n        pass\n    return {returned}\n",
            );
            common::assert_matches_cpython_marshal(&source, "pass_finalizer_copied_return.py");
        }
    }
}

#[test]
fn completed_value_join_preserves_later_manager_finally_owners() {
    for (definition, body) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
    ] {
        for value in [
            "value if enabled else None",
            "work() if value is not None else 0",
        ] {
            let source = format!(
                "{definition} f(manager, value, enabled):\n    with manager:\n        try:\n            {body}\n        except Error:\n            raise\n        finally:\n            if value:\n                item = {value}\n                with manager:\n                    work(item)\n",
            );
            common::assert_matches_cpython_marshal(
                &source,
                "completed_value_join_manager_finally.py",
            );
        }
    }
}

#[test]
fn completed_recovery_preserves_later_manager_finally_owners() {
    for (definition, body) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
    ] {
        for recovery in ["poll()", "while poll():\n    pass"] {
            let source = format!(
                "{definition} f(manager, value):\n    with manager:\n        try:\n            {body}\n        finally:\n            if value:\n                try:\n{}                except Error:\n                    pass\n                try:\n                    work()\n                except Error:\n                    pass\n",
                indent(&indent(&indent(&indent(&indent(&format!("{recovery}\n")))))),
            );
            common::assert_matches_cpython_marshal(
                &source,
                "completed_recovery_manager_finally.py",
            );
        }
    }
}

#[test]
fn folded_pass_join_preserves_the_authored_finally_marker() {
    for (header, prefix) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for continuation in ["return True", "consume(value)\nreturn value"] {
            let source = format!(
                "{header} f(value):\n{prefix}    try:\n        if True:\n            pass\n        else:\n            pass\n    finally:\n        pass\n{}",
                indent(&format!("{continuation}\n")),
            );
            common::assert_matches_cpython_marshal(&source, "folded_pass_finally_marker.py");
        }
    }
}
