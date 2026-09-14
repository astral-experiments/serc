use crate::common;

#[test]
fn matches_cpython_shared_assertion_handler_cleanup_locations() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield None\n"),
        ("async def", "    yield None\n"),
    ] {
        for handler in ["Error", "Error as error"] {
            for condition in ["a or b", "not (a and b)", "a or (b and c)"] {
                for successor in ["", "    consume(a)\n"] {
                    let source = format!(
                        "{function} f(a, b, c, message):\n{prefix}    try:\n        work()\n    except {handler}:\n        assert (\n            {condition}\n        ), message\n{successor}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "shared_assertion_handler_cleanup.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_folded_assertion_handler_cleanup_locations() {
    for function in ["def", "async def"] {
        for handler in ["Error", "Error as error"] {
            for condition in ["a or False", "False and (a or b)", "(a and b) or False"] {
                let source = format!(
                    "{function} f(a, b):\n    try:\n        work()\n    except {handler}:\n        assert {condition}\n    after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "folded_assertion_handler_cleanup.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_trailing_pass_return_copy_limit() {
    for (scope, prefix) in [
        ("", ""),
        ("async ", ""),
        ("", "    yield flag\n"),
        ("async ", "    yield flag\n"),
    ] {
        for branch in [
            "if flag:\n    work()",
            "if flag:\n    work()\nelse:\n    other()",
            "if flag:\n    work()\nelif other_flag:\n    middle()\nelse:\n    other()",
        ] {
            for count in 1..=3 {
                let source = format!(
                    "{scope}def process(flag, other_flag):\n{prefix}    {}\n{}",
                    branch.replace('\n', "\n    "),
                    "    pass\n".repeat(count),
                );
                common::assert_matches_cpython_marshal(&source, "trailing_pass_return.py");
            }
        }
    }
}

#[test]
fn matches_cpython_infinite_loop_handler_break_finally_locations() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for condition in ["True", "flag"] {
            for handler in ["Error", "Error as error"] {
                for cleanup in ["cleanup()", "manager.cleanup()"] {
                    let source = format!(
                        "{function} f(manager, flag):\n{prefix}    try:\n        while {condition}:\n            try:\n                step()\n            except {handler}:\n                break\n    finally:\n        {cleanup}\n    return\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "infinite_loop_handler_finally.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_exception_group_body_cleanup_locations() {
    for (scope, generator) in [("", false), ("async ", false), ("", true), ("async ", true)] {
        for managed in [false, true] {
            for handler in ["Error", "Error as error", "multiple"] {
                for body in [
                    "work(value)",
                    "while check(value):\n    work(value)",
                    "if value:\n    work(value)",
                ] {
                    let multiple = handler == "multiple";
                    let handler = if multiple { "Error" } else { handler };
                    let mut body = format!(
                        "try:\n    fail(value)\nexcept* {handler}:\n    {}\n",
                        body.replace('\n', "\n    ")
                    );
                    if multiple {
                        body.push_str("except* OtherError:\n    recover()\n");
                    }
                    if managed {
                        body = format!(
                            "with manager:\n    {}",
                            body.trim_end().replace('\n', "\n    ")
                        );
                        body.push('\n');
                    }
                    if generator {
                        body.push_str("yield None\n");
                    }
                    let source = format!(
                        "{scope}def process(value, manager):\n    {}\n",
                        body.trim_end().replace('\n', "\n    ")
                    );
                    common::assert_matches_cpython_marshal(&source, "exception_group_cleanup.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_handler_assertion_success_locations() {
    for (scope, generator) in [("", false), ("async ", false), ("", true), ("async ", true)] {
        for handler in ["Error", "Error as error"] {
            for condition in [
                "left and right",
                "not (left or right)",
                "left in values and right in values",
            ] {
                for message in ["", ", message"] {
                    let prefix = if generator { "    yield None\n" } else { "" };
                    let source = format!(
                        "{scope}def process(left, right, values, message):\n{prefix}    try:\n        fail()\n    except {handler}:\n        assert {condition}{message}\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "handler_assertion_success.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_handler_assertion_cleanup_locations() {
    for (filename, source) in [
        (
            "cleanup_reverse_control_assert_loop.py",
            r"def f(values, flag, other):
    for value in values:
        try:
            work(value)
        except Error as error:
            assert flag or other
        else:
            raise Other
",
        ),
        (
            "cleanup_assert_loop_else_call.py",
            r"def f(values, flag, other):
    for value in values:
        try:
            work(value)
        except Error as error:
            assert flag or other
        else:
            fail()
",
        ),
        (
            "cleanup_assert_loop_following_statement.py",
            r"def f(values, flag, other):
    for value in values:
        try:
            work(value)
        except Error as error:
            assert flag or other
        else:
            raise Other
        after()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_try_else_break_backedge_locations() {
    for (filename, scope, action, condition, following) in [
        ("try_else_break_name.py", "", "read(value)", "key", ""),
        (
            "try_else_break_none_async.py",
            "async ",
            "await read(value)",
            "key is None",
            "",
        ),
        (
            "try_else_break_compare.py",
            "",
            "read(value)",
            "len(key) == 0",
            "",
        ),
        (
            "try_else_break_multiple_false_control.py",
            "",
            "read(value)",
            "key and other",
            "",
        ),
    ] {
        let source = format!(
            "{scope}def f(values, other):\n    for value in values:\n        try:\n            key = {action}\n        except Error:\n            continue\n        else:\n            if {condition}:\n                break\n{following}"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_suspended_handler_cleanup_locations() {
    for (scope, action, prefix) in [
        ("async ", "await finish()", ""),
        ("", "yield value", ""),
        ("", "yield from values", ""),
        ("async ", "await finish()", "    yield value\n"),
    ] {
        for header in ["for item in values:", "while flag:"] {
            for handler in ["Error", "Error as error"] {
                for alternate in ["raise", "finish()"] {
                    for swapped in [false, true] {
                        let (body, orelse) = if swapped {
                            (alternate, action)
                        } else {
                            (action, alternate)
                        };
                        let source = format!(
                            "{scope}def process(flag, values, value):\n{prefix}    {header}\n        try:\n            work()\n        except {handler}:\n            if flag:\n                {body}\n            else:\n                {orelse}\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "suspended_handler_cleanup.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_loop_branch_exit_locations() {
    for loop_body in [
        "for item in values:\n    if predicate(item):\n        break",
        "for item in values:\n    consume(item)",
        "for item in values:\n    break",
        "for item in values:\n    if predicate(item):\n        break\nelse:\n    consume(values)",
        "while predicate(values):\n    consume(values)",
    ] {
        for alternate in ["consume(values)", "values = transform(values)", "pass"] {
            for successor in [
                "return values",
                "return None",
                "consume(flag)\nreturn values",
            ] {
                let loop_body = loop_body.replace('\n', "\n        ");
                let successor = successor.replace('\n', "\n    ");
                let source = format!(
                    "def classify(values, flag):\n    if flag:\n        {loop_body}\n    else:\n        {alternate}\n    {successor}\n"
                );
                common::assert_matches_cpython_marshal(&source, "loop_branch_exit_locations.py");
            }
        }
    }
}

#[test]
fn matches_cpython_compound_while_branch_exit_locations() {
    for condition in [
        "a and b",
        "a or b",
        "not (a or b)",
        "not (a and b)",
        "a and (b or c)",
        "a or (b and c)",
        "a < b < c",
        "a < b < c and d",
    ] {
        for successor in ["return a", "consume(a)\nreturn a"] {
            let successor = successor.replace('\n', "\n    ");
            let source = format!(
                "def f(flag, a, b, c, d):\n    if flag:\n        while {condition}:\n            consume(a)\n    else:\n        consume(b)\n    {successor}\n"
            );
            common::assert_matches_cpython_marshal(&source, "compound_while_branch_exit.py");
        }
    }
}

#[test]
fn matches_cpython_nested_comparison_branch_exit_locations() {
    for condition in ["a < b < c", "a is None is None"] {
        for action in ["consume(a)", "raise Error(a)"] {
            for successor in ["return a", "consume(a)\nreturn a"] {
                let successor = successor.replace('\n', "\n    ");
                let source = format!(
                    "def f(flag, a, b, c):\n    if flag:\n        if {condition}:\n            {action}\n    else:\n        consume(b)\n    {successor}\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_comparison_branch_exit.py");
            }
        }
    }
}

#[test]
fn matches_cpython_protected_loop_trailing_pass_locations() {
    for (function, loop_header) in [
        ("def", "for item in items:"),
        ("def", "while flag:"),
        ("async def", "for item in items:"),
        ("async def", "while flag:"),
        ("async def", "async for item in items:"),
    ] {
        for body in [
            "consume(item)\npass",
            "if flag:\n    consume(item)\nelse:\n    pass",
            "if flag:\n    consume(item)\nelif other:\n    consume(other)\nelse:\n    pass",
        ] {
            let body = body.replace('\n', "\n    ");
            let loop_body = format!("{loop_header}\n    {body}");
            for protected in [
                loop_body.clone(),
                format!(
                    "try:\n    {}\nexcept Error:\n    consume(flag)",
                    loop_body.replace('\n', "\n    ")
                ),
                format!("with manager:\n    {}", loop_body.replace('\n', "\n    ")),
            ] {
                let source = format!(
                    "{function} f(items, flag, other, manager):\n    {}\n    return flag\n",
                    protected.replace('\n', "\n    ")
                );
                common::assert_matches_cpython_marshal(&source, "loop_trailing_pass_locations.py");
            }
        }
    }
}

#[test]
fn matches_cpython_chained_comparison_exits_after_handler() {
    for function in ["def", "async def"] {
        for condition in ["0 <= value <= 1", "not 0 <= value <= 1"] {
            for action in ["return None", "return value", "raise Error(value)"] {
                let source = format!(
                    "{function} f(value):\n    try:\n        work()\n    except Error:\n        return None\n    if {condition}:\n        {action}\n    return transform(value)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "chained_comparison_after_handler.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_normal_finally_conditional_join_locations() {
    for function in ["def", "async def"] {
        for condition in ["check()", "flag"] {
            for action in ["work()", "value = work()"] {
                let source = format!(
                    "{function} f(flag):\n    if flag:\n        try:\n            {action}\n        finally:\n            if {condition}:\n                finish()\n    else:\n        alternate()\n    consume(flag)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "normal_finally_conditional_join.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_shared_assertion_success_branch_locations() {
    for function in ["def", "async def"] {
        for condition in ["a or b", "not a or not (b or c)", "(a and b) or c"] {
            for message in ["", ", message"] {
                for successor in ["consume(a)", "return a"] {
                    let source = format!(
                        "{function} f(flag, a, b, c, message):\n    if flag:\n        assert {condition}{message}\n    else:\n        consume(b)\n    {successor}\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "shared_assertion_success.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_elif_shared_exit_locations() {
    for function in ["def", "async def"] {
        for condition in ["a and b", "not (a or b)"] {
            for prior in ["return a", "consume(a)"] {
                for successor in ["consume(a)", "return a"] {
                    let source = format!(
                        "{function} f(flag, a, b, c):\n    if flag:\n        if c:\n            {prior}\n        elif {condition}:\n            return b\n    else:\n        consume(b)\n    {successor}\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_elif_shared_exit.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_context_manager_branch_exit_locations() {
    for (function, context_manager) in [
        ("def", "with manager"),
        ("async def", "with manager"),
        ("async def", "async with manager"),
    ] {
        for condition in ["a", "a or b"] {
            for successor in ["consume(b)", "return b"] {
                let source = format!(
                    "{function} f(flag, a, b, manager):\n    if flag:\n        if {condition}:\n            {context_manager}:\n                consume(a)\n    else:\n        consume(b)\n    {successor}\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_context_manager_exit.py");
            }
        }
    }
}

#[test]
fn matches_cpython_complete_finally_branch_exit_nops() {
    for (function, leading) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for branches in [
            "if flag:\n    recover()\nelse:\n    cleanup()",
            "if flag:\n    value = recover()\nelse:\n    del flag",
        ] {
            for continuation in ["after()", "return"] {
                let source = format!(
                    "{function} f(flag):\n{leading}    try:\n        work()\n    finally:\n        {}\n    {continuation}\n",
                    branches.replace('\n', "\n        ")
                );
                common::assert_matches_cpython_marshal(&source, "complete_finally_branch_exit.py");
            }
        }
    }
}

#[test]
fn matches_cpython_finally_context_manager_condition_exit_nops() {
    for continuation in [
        "return value",
        "return (value, 1, 2)",
        "return (value, 1, 2, 3, 4)",
        "transformed = value.attr()\nreturn consume(transformed, value)",
    ] {
        let source = format!(
            "def f(value):\n    try:\n        call(value)\n    finally:\n        if value:\n            with value:\n                finish()\n    {}\n",
            continuation.replace('\n', "\n    ")
        );
        common::assert_matches_cpython_marshal(
            &source,
            "finally_context_manager_condition_exit.py",
        );
    }
}
