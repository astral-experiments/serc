use crate::common;
use std::fmt::Write;

#[test]
fn matches_cpython_condition_owners_after_retained_setups() {
    for (scope, tail) in [
        ("", "    return rows\n"),
        ("async ", "    await finish()\n"),
        ("", "    yield obj\n"),
        ("async ", "    yield obj\n"),
    ] {
        for leading in [
            "            if not line:\n                continue\n",
            "            if line:\n                work()\n",
        ] {
            for condition in ["obj", "predicate(obj)", "test(obj)", "obj is None"] {
                let source = format!(
                    "{scope}def process(items, rows, obj, test):\n    try:\n        for line in items:\n{leading}            try:\n                if {condition}:\n                    rows.append(obj)\n            except Error:\n                continue\n    except Error:\n        recover()\n{tail}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "retained_setup_condition_owners.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_boolean_value_owner_across_try_entry() {
    for asynchronous in [false, true] {
        for closure in [false, true] {
            for guard in ["", "    if bypass():\n        return\n"] {
                for prefix in ["convert(x)", "wrap(convert(x))"] {
                    for condition in ["value", "value is not None", "value == x"] {
                        let scope = if asynchronous { "async " } else { "" };
                        let parameters = if closure { "value" } else { "x, value" };
                        let source = format!(
                            "{scope}def process({parameters}):\n{guard}    object_id = {prefix}\n    try:\n        value = prepare()\n        enabled = {condition} and check(value)\n    except Exception as error:\n        enabled = False\n    return x\n",
                        );
                        let source = if closure {
                            let body = source.lines().fold(String::new(), |mut output, line| {
                                writeln!(output, "    {line}").unwrap();
                                output
                            });
                            format!("def outer(x):\n{body}    return process\n")
                        } else {
                            source
                        };
                        common::assert_matches_cpython_marshal(
                            &source,
                            "try_entry_boolean_owner.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_exception_group_setup_and_merge_owners() {
    for asynchronous in [false, true] {
        for generator in [false, true] {
            let scope = if asynchronous { "async " } else { "" };
            let tail = if generator { "    yield None\n" } else { "" };
            for managed in [false, true] {
                let body = "try:\n    fail(value)\nexcept* Error:\n    work(value)\n";
                let body = if managed {
                    let entry = if asynchronous { "async with" } else { "with" };
                    format!(
                        "{entry} manager:\n{}",
                        body.lines().fold(String::new(), |mut indented, line| {
                            writeln!(indented, "    {line}").unwrap();
                            indented
                        }),
                    )
                } else {
                    body.to_owned()
                };
                let source = format!(
                    "{scope}def process(value, manager):\n{}{tail}",
                    body.lines().fold(String::new(), |mut indented, line| {
                        writeln!(indented, "    {line}").unwrap();
                        indented
                    }),
                );
                common::assert_matches_cpython_marshal(&source, "exception_group_merge_owner.py");
            }
        }
    }
}

#[test]
fn matches_cpython_assertion_success_comparison_owner_slots() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield"),
        ("async def", "await work()"),
        ("async def", "yield"),
    ] {
        for update in ["", "            count -= 1\n"] {
            for condition in ["count == 0", "predicate(count)", "count"] {
                let source = format!(
                    "{scope} process(manager):\n    global count, owned\n    try:\n        {body}\n    finally:\n        result = False\n        with manager:\n            assert count > 0\n{update}            if {condition}:\n                result = owned\n                owned = False\n        if result:\n            shutdown()\n",
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "assertion_success_comparison_owner.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_manager_condition_owners_when_entered_in_finally() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield"),
        ("async def", "await work()"),
        ("async def", "yield"),
    ] {
        for prior_manager in [false, true] {
            let prefix = if prior_manager {
                "    with manager:\n        if count == 0:\n            owned = acquire()\n        count += 1\n"
            } else {
                ""
            };
            for condition in [
                "count == 0",
                "predicate(count)",
                "count",
                "owned is not None",
            ] {
                let source = format!(
                    "{scope} process(manager):\n    global count, owned\n{prefix}    try:\n        {body}\n    finally:\n        with manager:\n            count -= 1\n            if {condition}:\n                assert owned is not None\n                close(owned)\n                owned = None\n",
                );
                common::assert_matches_cpython_marshal(&source, "manager_entered_in_finally.py");
            }
        }
    }
}

#[test]
fn matches_cpython_finally_manager_owner_excludes_nested_handlers() {
    for (scope, action) in [
        ("def", "work()"),
        ("def", "yield None"),
        ("async def", "await work()"),
        ("async def", "yield None"),
    ] {
        for conditional in [false, true] {
            let body = "with manager:\n    try:\n        work()\n    except Error:\n        if not value:\n            raise\n";
            let body = if conditional {
                format!(
                    "if ready():\n{}",
                    body.lines().fold(String::new(), |mut output, line| {
                        writeln!(output, "    {line}").unwrap();
                        output
                    }),
                )
            } else {
                body.to_owned()
            };
            let source = format!(
                "{scope} process(value, manager):\n    try:\n        {action}\n    finally:\n{}",
                body.lines().fold(String::new(), |mut output, line| {
                    writeln!(output, "        {line}").unwrap();
                    output
                }),
            );
            common::assert_matches_cpython_marshal(&source, "finally_manager_nested_handler.py");
        }
    }
}

#[test]
fn matches_cpython_terminal_if_body_owner_slots() {
    for initial in [
        "    await prepare()\n",
        "    async with manager():\n        pass\n",
    ] {
        for outer in ["flag", "not obj.ready"] {
            for condition in [
                "ready(obj)",
                "local(obj)",
                "obj.ready()",
                "ready(obj) and other(obj)",
            ] {
                for body in ["raise Error()", "return 1", "work()"] {
                    let source = format!(
                        "async def process(flag, obj, local):\n{initial}    if {outer}:\n        if {condition}:\n            {body}\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "terminal_if_body_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_condition_body_owner_boundaries() {
    for scope in ["", "async "] {
        for outer in ["flag", "flag and other"] {
            for prefix in [
                "",
                "            obj = manager.get()\n",
                "            work()\n",
            ] {
                for inner in ["obj", "obj.ready"] {
                    let source = format!(
                        "{scope}def process(flag, other, obj, manager):\n    if {outer}:\n        try:\n{prefix}            if {inner}:\n                work()\n        except Error:\n            recover()\n    yield obj\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "condition_body_owners.py");
                }
            }
        }
    }
    for prefix in [
        "",
        "                obj = manager.get()\n",
        "                work()\n",
    ] {
        for inner in ["obj", "obj.ready"] {
            let source = format!(
                "def outer(send):\n    async def process(flag, obj, manager):\n        if flag:\n            try:\n{prefix}                if {inner}:\n                    work()\n            except Error:\n                pass\n        await send(obj)\n    return process\n"
            );
            common::assert_matches_cpython_marshal(&source, "closure_condition_body_owners.py");
        }
    }
}

#[test]
fn matches_cpython_original_short_circuit_while_condition_owners() {
    for asynchronous in [false, true] {
        for generator in [false, true] {
            let scope = if asynchronous { "async " } else { "" };
            let tail = if generator { "    yield None\n" } else { "" };
            for condition in [
                "ready() and value",
                "value and ready()",
                "ready() or obj.ready()",
                "not (ready() and value)",
                "obj.ready(value) and value is not None",
                "ready() and obj.ready() and value",
            ] {
                for context in ["try", "with"] {
                    let (entry, cleanup) = if context == "try" {
                        ("try:", "    except Error:\n        recover()\n")
                    } else if asynchronous {
                        ("async with manager:", "")
                    } else {
                        ("with manager:", "")
                    };
                    let source = format!(
                        "{scope}def process(value, obj, manager):\n    {entry}\n        while {condition}:\n            work()\n            value = update()\n        finish(value)\n{cleanup}{tail}",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "short_circuit_while_owners.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_generator_loop_guard_owner_controls() {
    let source = r"def optimized_generator_controls(items):
    while all(item for item in items):
        yield items
    while any(item for item in items):
        yield items
    while tuple(item for item in items):
        yield items


def unary_not_control(item):
    while not should_continue(item):
        yield item
";
    common::assert_matches_cpython_marshal(source, "generator_loop_guard_owners.py");
}

#[test]
fn matches_cpython_negated_original_while_condition_owners() {
    for asynchronous in [false, true] {
        for generator in [false, true] {
            let scope = if asynchronous { "async " } else { "" };
            let tail = if generator { "    yield None\n" } else { "" };
            for condition in [
                "not ready()",
                "not len(obj.values)",
                "not obj.ready()",
                "not not ready()",
                "not (value is None)",
            ] {
                for context in ["try", "with"] {
                    let (entry, cleanup) = if context == "try" {
                        ("try:", "    except Error:\n        recover()\n")
                    } else if asynchronous {
                        ("async with manager:", "")
                    } else {
                        ("with manager:", "")
                    };
                    let source = format!(
                        "{scope}def process(value, obj, manager):\n    {entry}\n        while {condition}:\n            work()\n            value = update()\n        finish(value)\n{cleanup}{tail}",
                    );
                    common::assert_matches_cpython_marshal(&source, "negated_while_owners.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_value_conditional_branch_owner_boundaries() {
    for asynchronous in [false, true] {
        for generator in [false, true] {
            let scope = if asynchronous { "async " } else { "" };
            let tail = if generator { "    yield None\n" } else { "" };
            let work = if asynchronous {
                "await work()"
            } else {
                "work()"
            };
            for expression in [
                "value if flag else (1 if other else 2)",
                "(1 if other else 2) if flag else value",
                "value if flag else (1 if check(other) else 2)",
                "(1 if check(other) else 2) if flag else value",
                "value if flag else (1 if other is None else 2)",
                "value if value is not None else (1 if other else 2)",
            ] {
                let source = format!(
                    "{scope}def process(value, flag, other):\n    try:\n        {work}\n    finally:\n        result = {expression}\n        use(result)\n{tail}",
                );
                common::assert_matches_cpython_marshal(&source, "value_conditional_owners.py");
            }
        }
    }
}

#[test]
fn matches_cpython_prologue_consumes_condition_owner_slots() {
    for cell in ["", "        def capture():\n            return y\n"] {
        for prefix in ["", "        record()\n"] {
            for condition in ["y == 0", "get(y) == 0", "y is x"] {
                for assertion in [false, true] {
                    let condition = if assertion {
                        format!("            assert {condition}\n")
                    } else {
                        format!("            if {condition}:\n                return x\n")
                    };
                    let source = format!(
                        "def outer(x):\n    def process(y):\n{cell}{prefix}        try:\n{condition}        except Error:\n            pass\n        return x\n    return process\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "prologue_owner_slots.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_original_optimized_dispatch_condition_owner_slots() {
    for asynchronous in [false, true] {
        for generator in [false, true] {
            let prefix = if asynchronous { "async " } else { "" };
            let tail = if generator { "    yield None\n" } else { "" };
            for handler in ["Error", "Error as error"] {
                for initial in ["", "        record()\n"] {
                    for producer in [
                        "all(item for item in items)",
                        "any(check(item) for item in items)",
                        "tuple(item.attr for item in items)",
                    ] {
                        let source = format!(
                            "{prefix}def process(items):\n    try:\n        work()\n    except {handler}:\n{initial}        result = {producer}\n        consume(result)\n{tail}",
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "original_dispatch_owner.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_retained_setup_after_local_fusion_condition_owners() {
    for assignments in [3, 5, 6, 10] {
        let prefix = "    x = y = z = 1\n".repeat(assignments);
        for condition in ["call() is x", "y is x", "call() in collection"] {
            for assertion in [false, true] {
                let condition = if assertion {
                    format!("        assert {condition}\n")
                } else {
                    format!("        if {condition}:\n            work()\n")
                };
                let source = format!(
                    "def process(x, y, z, collection):\n{prefix}    try:\n{condition}    finally:\n        pass\n",
                );
                common::assert_matches_cpython_marshal(&source, "local_fusion_condition_owner.py");
            }
        }
    }
}

#[test]
fn matches_cpython_protected_while_none_condition_owner_slots() {
    for asynchronous in [false, true] {
        for generator in [false, true] {
            let prefix = if asynchronous { "async " } else { "" };
            let tail = if generator { "    yield value\n" } else { "" };
            for initial in ["", "        value = prepare()\n"] {
                for condition in [
                    "value is None",
                    "value is not None",
                    "obj.value is not None",
                    "ready() is not None",
                ] {
                    for cleanup in [
                        "    finally:\n        cleanup()\n",
                        "    except Error:\n        recover()\n",
                    ] {
                        let source = format!(
                            "{prefix}def process(value, obj):\n    try:\n{initial}        while {condition}:\n            work()\n            value = update()\n{cleanup}{tail}",
                        );
                        common::assert_matches_cpython_marshal(&source, "while_none_owner.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_retained_setup_first_compaction_condition_owners() {
    for prefix in [
        "    record()\n".repeat(5),
        "    record()\n".repeat(6),
        "    y = 1 + 2 + 3 + 4\n".to_owned(),
    ] {
        for condition in [
            "call() is x",
            "y is x",
            "call() in collection",
            "x in collection",
        ] {
            for assertion in [false, true] {
                for cleanup in [
                    "    finally:\n        pass\n",
                    "    except Error:\n        recover()\n",
                ] {
                    let condition = if assertion {
                        format!("        assert {condition}\n")
                    } else {
                        format!("        if {condition}:\n            work()\n")
                    };
                    let source = format!(
                        "def process(x, y, collection):\n{prefix}    try:\n{condition}{cleanup}",
                    );
                    common::assert_matches_cpython_marshal(&source, "first_compaction_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_first_setup_condition_owners_after_global_calls() {
    // Each global call removes a PUSH_NULL slot. Four calls move the inserted marker
    // from the protected condition suffix into the original unowned SETUP record.
    for prefix_calls in [0, 3, 4, 5, 16] {
        let prefix = "    prepare()\n".repeat(prefix_calls);
        for condition in ["flag", "not flag"] {
            let source = format!(
                "def f(flag):\n{prefix}    try:\n        if {condition}:\n            action()\n    finally:\n        cleanup()\n",
            );
            common::assert_matches_cpython_marshal(&source, "first_setup_condition_owner.py");
        }
    }

    for prefix_calls in [0, 4, 8] {
        let prefix = "        prepare()\n".repeat(prefix_calls);
        let source = format!(
            "def f(flag, manager):\n    with manager():\n{prefix}        try:\n            if not flag:\n                action()\n        finally:\n            cleanup()\n",
        );
        common::assert_matches_cpython_marshal(&source, "outer_with_setup_condition_owner.py");
    }
}

#[test]
fn matches_cpython_async_for_first_condition_owner_slots() {
    for tail in ["consume(item)", "yield item"] {
        for initial in ["", "    await prepare()\n"] {
            for target in ["item", "(item, other)"] {
                for condition in ["flag and ready(item)", "obj.ready"] {
                    for body in ["work()", "continue"] {
                        let source = format!(
                            "async def process(items, item, other, flag, obj):\n{initial}    async for {target} in items:\n        if {condition}:\n            {body}\n        {tail}\n",
                        );
                        common::assert_matches_cpython_marshal(&source, "async_for_first_owner.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_structural_pattern_marker_owners() {
    for scope in ["", "async "] {
        for prefix in ["", "        work()\n"] {
            for (pattern, tail) in [
                ("[]", ""),
                ("['a']", ""),
                ("{}", ""),
                ("{'a': 1}", ""),
                ("['a']", "    yield value\n"),
            ] {
                let source = format!(
                    "{scope}def process(value):\n    try:\n{prefix}        match value:\n            case {pattern}:\n                result()\n    except Error:\n        recover()\n{tail}"
                );
                common::assert_matches_cpython_marshal(&source, "structural_pattern_owners.py");
            }
        }
    }
}

#[test]
fn matches_cpython_threaded_match_condition_owner_slots() {
    for initial in ["", "    await prepare()\n"] {
        for prefix in ["", "            record()\n"] {
            for condition in ["value", "value is None"] {
                for body in ["return None", "return value", "raise Error(value)"] {
                    let source = format!(
                        "async def process(kind, value):\n{initial}    match kind:\n        case 'first':\n{prefix}            if {condition}:\n                {body}\n        case 'second':\n            consume(value)\n        case _:\n            pass\n    consume(value)\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "threaded_match_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_if_expression_condition_owner_slots() {
    for initial in ["await prepare()", "value = await prepare()"] {
        for prefix in ["", "    record()\n"] {
            for condition in [
                "ready() if flag else other",
                "flag if other else ready()",
                "ready() if flag else other_ready()",
            ] {
                let source = format!(
                    "async def process(flag, other, value):\n    {initial}\n{prefix}    if {condition}:\n        await work()\n",
                );
                common::assert_matches_cpython_marshal(&source, "terminal_if_expression_owner.py");
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_condition_owner_slots_after_loop_exits() {
    for initial in ["await prepare()", "value = await prepare()"] {
        for loop_body in [
            "    for item in items:\n        work()\n",
            "    while flag:\n        flag = work()\n",
        ] {
            for prefix in ["", "    record()\n"] {
                for condition in ["ready()", "obj.ready", "sum(item for item in items)"] {
                    let source = format!(
                        "async def process(flag, value, obj, items):\n    {initial}\n{loop_body}{prefix}    if {condition}:\n        await work()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "terminal_loop_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_condition_owner_slots_after_with_joins() {
    for initial in ["", "    await prepare()\n"] {
        for with_kind in ["with", "async with"] {
            for prefix in ["", "    record()\n"] {
                for condition in ["flag", "obj.ready", "ready()", "obj.ready()"] {
                    let source = format!(
                        "async def process(flag, obj, manager):\n{initial}    {with_kind} manager:\n        work()\n{prefix}    if {condition}:\n        await work()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "terminal_with_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_condition_owner_slots_after_await_calls() {
    for initial in ["await work()", "value = await work()"] {
        for prefix in ["record()", "value = work()", "value = (1, 2, 3, 4)"] {
            for condition in ["flag", "obj.ready", "ready()", "obj.ready()"] {
                let source = format!(
                    "async def process(flag, value, obj):\n    {initial}\n    {prefix}\n    if {condition}:\n        await work()\n",
                );
                common::assert_matches_cpython_marshal(&source, "terminal_await_owner.py");
            }
        }
        for prefix in ["", "    obj.record()\n"] {
            let source = format!(
                "async def process(value, obj):\n    {initial}\n{prefix}    if ready():\n        await work()\n",
            );
            common::assert_matches_cpython_marshal(&source, "terminal_await_condition_owner.py");
        }
        // An expression branch starts new blocks after the prefix's original slot proof.
        let source = format!(
            "async def process(flag, other, value):\n    {initial}\n    record()\n    if ready() if flag else other:\n        await work()\n",
        );
        common::assert_matches_cpython_marshal(&source, "terminal_await_branch_owner.py");
    }
}

#[test]
fn matches_cpython_try_else_owner_slots_after_body_joins() {
    for scope in ["", "async "] {
        for body in [
            "        if flag:\n            work()\n",
            "        if flag:\n            if other:\n                work()\n",
            "        if flag:\n            work()\n        else:\n            other_work()\n",
        ] {
            for condition in ["value", "ready(value)"] {
                for second_handler in ["", "    except Other as error:\n        recover()\n"] {
                    let source = format!(
                        "{scope}def process(flag, other, value):\n    try:\n{body}    except Error:\n        recover()\n{second_handler}    else:\n        if {condition}:\n            work()\n",
                    );
                    common::assert_matches_cpython_marshal(&source, "try_else_owner_join.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_try_else_owner_slots_after_suspension() {
    for prefix in ["", "    await prepare()\n"] {
        for body in [
            "        if flag:\n            work()\n",
            "        await work()\n",
            "        if flag:\n            await work()\n",
            "        if flag:\n            await work()\n        else:\n            work()\n",
        ] {
            for condition in ["value", "value.ready", "value.ready()", "value[index]"] {
                let source = format!(
                    "async def process(flag, value, index):\n{prefix}    try:\n{body}    except Error:\n        recover()\n    else:\n        if {condition}:\n            work()\n",
                );
                common::assert_matches_cpython_marshal(&source, "terminal_try_else_owner.py");
            }
        }
    }
}

#[test]
fn matches_cpython_late_with_setup_suffix_owners() {
    for (scope, initial) in [
        ("", "work()"),
        ("", "yield obj"),
        ("async ", "await work()"),
        ("async ", "yield obj"),
    ] {
        for prefix in ["record()", "value = (1, 2, 3, 4, 5, 6, 7, 8, 9, 10)"] {
            for condition in ["flag", "obj.exists()", "obj is not None"] {
                let source = format!(
                    "{scope}def case(obj, flag):\n    try:\n        {initial}\n        {prefix}\n    finally:\n        with manager():\n            if {condition}:\n                work()\n",
                );
                common::assert_matches_cpython_marshal(&source, "late_with_setup_suffix.py");
            }
        }
    }

    // A later setup, cleanup owner transition, or source join ends the original suffix.
    for (scope, initial) in [("", "yield obj"), ("async ", "await work()")] {
        for body in [
            "        try:\n            if flag:\n                work()\n        except Error:\n            pass\n",
            "        try:\n            work()\n        finally:\n            if flag:\n                work()\n",
            "        with manager():\n            if flag:\n                work()\n",
            "        if flag:\n            work()\n        if flag:\n            work()\n",
        ] {
            let source =
                format!("{scope}def case(obj, flag):\n    {initial}\n    with manager():\n{body}");
            common::assert_matches_cpython_marshal(&source, "late_with_setup_suffix_boundary.py");
        }
    }
}

#[test]
fn matches_cpython_class_pattern_owner_slots_after_source_joins() {
    for (scope, resume) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for first in [
            "    if flag:\n        prepare()\n",
            "    value = left if flag else right\n",
        ] {
            for pattern in [
                "First()",
                "First(item)",
                "First(key=item)",
                "First() | Other()",
            ] {
                for protected in [false, true] {
                    let mut source =
                        format!("{scope}def process(value, flag, left, right):\n{resume}{first}");
                    let indent = if protected {
                        source.push_str("    try:\n");
                        "        "
                    } else {
                        "    "
                    };
                    for line in format!(
                        "match value:\n    case {pattern}:\n        return 1\n    case Other():\n        return 2\n    case _:\n        return 3",
                    )
                    .lines()
                    {
                        writeln!(source, "{indent}{line}").unwrap();
                    }
                    if protected {
                        source.push_str("    except Error:\n        recover()\n");
                    }
                    common::assert_matches_cpython_marshal(&source, "class_pattern_owner_slots.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_condition_owners_after_retained_setup_nop() {
    for (scope, first) in [
        ("", "    if flag:\n        prepare()\n"),
        ("async ", "    if flag:\n        prepare()\n"),
        ("async ", "    await prepare()\n"),
    ] {
        for prefix in ["", "    alias = obj.name\n"] {
            for condition in ["value", "obj.ready(value)"] {
                for cleanup in [
                    "    except Error:\n        recover()\n",
                    "    finally:\n        cleanup()\n",
                ] {
                    let source = format!(
                        "{scope}def process(flag, obj, value):\n{first}{prefix}    try:\n        if {condition}:\n            work()\n{cleanup}",
                    );
                    common::assert_matches_cpython_marshal(&source, "setup_nop_owner_slots.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_forward_loop_condition_source_boundaries() {
    for (scope, value, calls) in [
        ("async ", "await request(image)", 0),
        ("async ", "await request(image)", 5),
        ("async ", "await request(image)", 7),
        ("", "image", 5),
        ("", "yield image", 7),
    ] {
        for condition in ["response is None", "response is not None"] {
            let mut source = format!("{scope}def process(image):\n    try:\n        while True:\n");
            for index in 0..calls {
                writeln!(source, "            prepared{index} = prepare()").unwrap();
            }
            writeln!(
                source,
                "            try:\n                response = {value}\n                if {condition}:\n                    failed()\n                else:\n                    succeeded()\n            except Error:\n                failed()\n    except Cancelled:\n        return",
            )
            .unwrap();
            common::assert_matches_cpython_marshal(&source, "resumed_loop_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_forward_loop_condition_owners_after_compaction() {
    for (scope, value) in [
        ("", "image"),
        ("async ", "await request(image)"),
        ("", "yield image"),
    ] {
        for prefix in [
            "",
            "                record()\n",
            "                stats.record(image)\n",
            "                prepared = (1, 2, 3, 4, 5)\n",
        ] {
            for condition in ["response", "not response"] {
                let source = format!(
                    "{scope}def process(image):\n    try:\n        while True:\n            try:\n                response = {value}\n{prefix}                if {condition}:\n                    failed()\n                else:\n                    succeeded()\n            except Error:\n                failed()\n    except Cancelled:\n        return\n",
                );
                common::assert_matches_cpython_marshal(&source, "forward_loop_owner_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_boolean_owner_slots_after_unprotected_prefixes() {
    for calls in [0, 2, 3, 4, 5, 6, 7, 9] {
        for value in [
            "flag or other",
            "int(flag or other)",
            "json.loads(flag or other)",
        ] {
            let mut source = String::from("def process(flag, other):\n");
            for index in 0..calls {
                writeln!(source, "    value{index} = prepare()").unwrap();
            }
            writeln!(
                source,
                "    try:\n        result = {value}\n    except Error:\n        return None\n    return result",
            )
            .unwrap();
            common::assert_matches_cpython_marshal(&source, "unprotected_prefix_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_owner_slots_after_large_constant_folds() {
    for calls in [0, 1, 3] {
        for values in ["('a',)", "('a', 'b', 'c')", "('a', 'b', 'c', 'd', 'e')"] {
            let mut source = String::from("def process(flag, other):\n");
            for index in 0..calls {
                writeln!(source, "    value{index} = prepare()").unwrap();
            }
            writeln!(
                source,
                "    prepared = prepare({values})\n    try:\n        result = json.loads(flag or other)\n    except Error:\n        return None\n    return result",
            )
            .unwrap();
            common::assert_matches_cpython_marshal(&source, "large_fold_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_ordinary_condition_slots_after_constant_folding() {
    for size in ["256", "16 * 16"] {
        for manager in ["with manager:", "with open(path) as stream:"] {
            let source = format!(
                "def process(path, manager, stream):\n    result = build()\n    {manager}\n        while chunk := stream.read({size}):\n            result.update(chunk)\n    return result\n",
            );
            common::assert_matches_cpython_marshal(&source, "ordinary_condition_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_tuple_dispatch_owners_after_protected_prefix() {
    for (scope, suspension, tail) in [
        ("def", "before()", ""),
        ("def", "yield None", "    yield None\n"),
        ("async def", "await before()", ""),
        ("async def", "await before()", "    yield None\n"),
    ] {
        for suspended in [false, true] {
            let prefix = if suspended {
                format!("    {suspension}\n")
            } else {
                String::new()
            };
            for preparation in ["", "        temp = prepare()\n"] {
                for following in ["", "        use(values)\n"] {
                    let source = format!(
                        "{scope} process(items):\n{prefix}    try:\n{preparation}        values = tuple(item for item in items)\n{following}    except Error:\n        recover()\n{tail}",
                    );
                    common::assert_matches_cpython_marshal(&source, "tuple_dispatch_try_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_folded_while_body_owner_boundary() {
    for (scope, tests) in [
        ("module", &["True", "1", "not False", "1 + 2"][..]),
        ("sync", &["True", "1", "not False", "1 + 2"][..]),
        ("generator", &["True", "1"][..]),
        ("coroutine", &["True", "1"][..]),
    ] {
        for test in tests {
            for prefix in ["", "        value = read()\n"] {
                for following in ["", "after()\n"] {
                    let body = format!(
                        "while {test}:\n    try:\n{prefix}        if not value:\n            continue\n        consume(value)\n    except Error:\n        break\n{following}",
                    );
                    let source = if scope == "module" {
                        body
                    } else {
                        let prefix = if scope == "coroutine" { "async " } else { "" };
                        let body = format!("    {}\n", body.trim_end().replace('\n', "\n    "));
                        let tail = if scope == "generator" {
                            "    yield None\n"
                        } else {
                            ""
                        };
                        format!("{prefix}def process(value):\n{body}{tail}")
                    };
                    common::assert_matches_cpython_marshal(&source, "folded_while_body_owner.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_later_manager_conditions_in_normal_finally() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield obj"),
        ("async def", "await work()"),
        ("async def", "yield obj"),
    ] {
        for condition in ["obj.second", "check_second()", "obj.second is None"] {
            for nested in [false, true] {
                let branch = if nested {
                    format!(
                        "            else:\n                if {condition}:\n                    other()\n"
                    )
                } else {
                    format!("            elif {condition}:\n                other()\n")
                };
                let source = format!(
                    "{scope} process(manager, obj):\n    try:\n        {body}\n    finally:\n        with manager:\n            if obj.first:\n                action()\n{branch}            after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "later_manager_finally_condition.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_legacy_try_loop_tail_source_owner_slots() {
    for (scope, manager, body) in [
        ("def", "with", "yield first"),
        ("async def", "with", "yield first"),
        ("async def", "async with", "yield first"),
        ("async def", "async with", "await consume(first)"),
    ] {
        for condition in [
            "first",
            "first and first not in seen",
            "first or first not in seen",
        ] {
            for prefix in ["first = decode(first)", "scratch = obj.method(first)"] {
                let source = format!(
                    "{scope} process(first, seen, obj, manager):\n    {manager} manager:\n        while True:\n            try:\n                if not seen:\n                    break\n                {prefix}\n                if {condition}:\n                    {body}\n            except Error:\n                break\n"
                );
                common::assert_matches_cpython_marshal(&source, "legacy_try_loop_source_owner.py");
            }
        }
    }
}

#[test]
fn matches_cpython_initial_while_comparison_owners() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield n"),
        ("async def", "await work()"),
        ("async def", "yield n"),
    ] {
        for condition in [
            "n == limit",
            "n != limit",
            "n < limit",
            "n <= limit",
            "n > limit",
            "n >= limit",
            "len(items) < limit",
            "clock() < limit",
        ] {
            let source = format!(
                "{scope} process(n, limit, items):\n    try:\n        while {condition}:\n            {body}\n            n += 1\n    except Error:\n        recover()\n    finally:\n        cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "initial_while_comparison_owners.py");
        }
    }
}

#[test]
fn matches_cpython_initial_while_identity_and_membership_owners() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield n"),
        ("async def", "await work()"),
        ("async def", "yield n"),
    ] {
        for condition in [
            "n is limit",
            "n is not limit",
            "n in items",
            "n not in items",
            "n is get_limit()",
            "len(items) is limit",
            "n in [1, 2, 3]",
            "n not in [1, 2, 3]",
            "n in {1, 2, 3}",
            "None is n",
            "None is not n",
            "n is None",
            "n is not None",
            "not (n in items)",
            "not (n is limit)",
        ] {
            let source = format!(
                "{scope} process(n, limit, items):\n    try:\n        while {condition}:\n            {body}\n            n += 1\n    except Error:\n        recover()\n    finally:\n        cleanup()\n"
            );
            common::assert_matches_cpython_marshal(&source, "initial_while_identity_owners.py");
        }
    }
}

#[test]
fn matches_cpython_manager_owner_after_nested_finally_pop_block() {
    for (scope, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for prefix in ["", "            n -= 1\n"] {
            for condition in ["n", "obj.n", "n == 0", "n in items"] {
                let source = format!(
                    "{scope} process(manager, n, obj, items):\n    {manager} manager:\n        try:\n            yield\n        finally:\n{prefix}            if {condition}:\n                after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "manager_owner_after_nested_finally.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_manager_finally_suffix_stops_at_source_joins() {
    for (prefix, condition) in [
        ("", "any(item for item in items)"),
        (
            "            while flag:\n                work()\n                break\n",
            "n and flag",
        ),
        (
            "            with other_manager:\n                work()\n",
            "n and flag",
        ),
    ] {
        let source = format!(
            "def process(managers, other_manager, items, obj, n, limit, flag):\n    with managers[0]:\n        try:\n            work()\n        finally:\n{prefix}            if {condition}:\n                after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "manager_finally_suffix_source_joins.py");
    }
}

#[test]
fn matches_cpython_early_finally_owner_after_await() {
    for manager in ["with", "async with"] {
        for prefix in ["", "            work()\n", "            n -= 1\n"] {
            for condition in ["n", "obj.n", "n == 0", "n and limit"] {
                let source = format!(
                    "async def process(manager, n, obj, limit):\n    {manager} manager:\n        try:\n            await work()\n        finally:\n{prefix}            if {condition}:\n                after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "early_finally_owner_after_await.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_finally_owner_after_local_fusion() {
    for manager in ["with", "async with"] {
        for prefix in [
            "a = b = n",
            "a, b = n, limit",
            "a = n; b = limit",
            "a = n + limit",
        ] {
            for condition in ["n", "n == 0", "n and limit"] {
                let source = format!(
                    "async def process(manager, n, limit):\n    {manager} manager:\n        try:\n            await work()\n        finally:\n            {prefix}\n            if {condition}:\n                after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "finally_owner_after_local_fusion.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_suspended_while_none_condition_owner_slots() {
    for (prefix, expression) in [("async ", "await next_value()"), ("", "yield value")] {
        for comparison in ["is None", "is not None"] {
            for assignment in ["", "received := "] {
                for cleanup in [
                    "    except Error:\n        recover()\n",
                    "    finally:\n        finish()\n",
                ] {
                    let source = format!(
                        "{prefix}def process(value):\n    try:\n        while ({assignment}({expression})) {comparison}:\n            consume(value)\n{cleanup}",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "suspended_while_none_owner.py",
                    );
                }
            }
        }
        // Bare suspended truth tests keep their existing owner rules; they do not
        // remove comparison instructions.
        let source = format!(
            "{prefix}def process(value):\n    try:\n        while ({expression}):\n            consume(value)\n    except Error:\n        recover()\n",
        );
        common::assert_matches_cpython_marshal(&source, "suspended_while_unowned_marker.py");
    }
}

#[test]
fn matches_cpython_runtime_while_finally_assignment_owners() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield x"),
        ("async def", "await work()"),
        ("async def", "yield x"),
    ] {
        for handler in [
            "except Error:\n        recover()",
            "except Error as error:\n        recover(error)",
        ] {
            for cleanup in [
                "x.running = False",
                "sink[index] = x",
                "sink.left, sink.right = x, other",
                "sink.left, *sink.rest = x",
                "sink.left = sink.right = x",
                "x = sink.value = x",
                "x, other = other, x",
                "sink.value = transform(x)",
                "consume(x)",
            ] {
                let source = format!(
                    "{scope} f(x, flag, other, sink, index, manager):\n    try:\n        while flag:\n            {body}\n    {handler}\n    finally:\n        {cleanup}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "runtime_while_finally_assignment_owners.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_runtime_while_finally_handler_shadow_owners() {
    for cleanup in [
        "x.running = False",
        "sink[index] = x",
        "sink.left, sink.right = x, other",
        "sink.left, *sink.rest = x",
        "sink.left = sink.right = x",
        "x = sink.value = x",
        "x, other = other, x",
        "sink.value = transform(x)",
        "consume(x)",
    ] {
        let source = format!(
            "def f(x, flag, other, sink, index, manager):\n    try:\n        while flag:\n            work()\n    except Error as x:\n        recover(x)\n    finally:\n        {cleanup}\n"
        );
        common::assert_matches_cpython_marshal(&source, "runtime_while_finally_shadow_owners.py");
    }
}

#[test]
fn matches_cpython_runtime_while_finally_borrow_blockers() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield x"),
        ("async def", "await work()"),
        ("async def", "yield x"),
    ] {
        for (condition, handler) in [
            ("flag", "except Error:\n        raise"),
            ("flag", "except:\n        pass"),
            ("flag and other", "except:\n        pass"),
        ] {
            let source = format!(
                "{scope} f(x, flag, other, sink, index, manager):\n    try:\n        while {condition}:\n            {body}\n    {handler}\n    finally:\n        sink.value = x\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "runtime_while_finally_borrow_blockers.py",
            );
        }
    }
}

#[test]
fn matches_cpython_runtime_while_finally_suspended_assignment_owners() {
    for (scope, body, cleanup) in [
        ("async def", "await work()", "x.value = await get()"),
        ("async def", "await work()", "(await get()).value = x"),
        ("async def", "await work()", "sink[await get()] = x"),
        ("def", "work()", "x.value = (yield other)"),
        ("def", "work()", "(yield other).value = x"),
        ("def", "work()", "sink[(yield index)] = x"),
    ] {
        let source = format!(
            "{scope} f(x, flag, other, sink, index, manager):\n    try:\n        while flag:\n            {body}\n    except Error as error:\n        recover(error)\n    finally:\n        {cleanup}\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "runtime_while_finally_suspended_assignments.py",
        );
    }
}

#[test]
fn matches_cpython_runtime_while_finally_assignment_target_branch() {
    for handler in [
        "except Error:\n        recover()",
        "except Error as error:\n        recover(error)",
    ] {
        let source = format!(
            "def f(x, flag, other, sink, index, manager):\n    try:\n        while flag:\n            work()\n    {handler}\n    finally:\n        sink[flag and index] = x\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "runtime_while_finally_assignment_target_branch.py",
        );
    }
}

#[test]
fn matches_cpython_constant_loop_inner_try_finally_cleanup() {
    for (scope, body) in [
        ("def", "work()"),
        ("def", "yield other"),
        ("async def", "await work()"),
        ("async def", "yield other"),
    ] {
        for handler in [
            "",
            "    except OuterError as error:\n        recover(error)\n",
        ] {
            for cleanup in [
                "consume(x)",
                "x.running = False",
                "if other:\n            consume(x)",
            ] {
                let source = format!(
                    "{scope} f(x, flag, other):\n    try:\n        while True:\n            try:\n                {body}\n            except Error:\n                pass\n            if flag:\n                break\n            work()\n{handler}    finally:\n        {cleanup}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "constant_loop_inner_try_finally_cleanup.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_constant_loop_inner_try_finally_suspension() {
    for (scope, body, cleanup) in [
        (
            "async def",
            "value = await work()\n                consume(value)",
            "await finish(x)",
        ),
        (
            "def",
            "value = yield other\n                consume(value)",
            "yield x",
        ),
        ("async def", "yield other", "yield x"),
    ] {
        let source = format!(
            "{scope} f(x, flag, other):\n    try:\n        while True:\n            try:\n                {body}\n            except Error:\n                pass\n            value = predicate(other)\n            if value == other:\n                break\n            work()\n    except OuterError as error:\n        recover(error)\n    finally:\n        {cleanup}\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "constant_loop_inner_try_finally_suspension.py",
        );
    }
}

#[test]
fn matches_cpython_constant_loop_inner_try_finally_blockers() {
    for (body, handler, suffix) in [
        (
            "work()",
            "except:\n                pass",
            "if flag:\n                break\n            work()",
        ),
        (
            "work()",
            "except Error:\n                raise",
            "if flag:\n                break\n            work()",
        ),
        (
            "work()",
            "except Error:\n                continue",
            "if flag:\n                break\n            work()",
        ),
        (
            "work()",
            "except Error:\n                return",
            "if flag:\n                break\n            work()",
        ),
        (
            "if other:\n                    work()",
            "except Error:\n                pass",
            "if flag:\n                break\n            work()",
        ),
        (
            "work()",
            "except Error:\n                pass",
            "if flag:\n                break",
        ),
    ] {
        let source = format!(
            "def f(x, flag, other):\n    try:\n        while True:\n            try:\n                {body}\n            {handler}\n            {suffix}\n    finally:\n        consume(x)\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "constant_loop_inner_try_finally_blockers.py",
        );
    }
}

#[test]
fn first_try_condition_markers_keep_original_block_owners() {
    for (filename, source) in [
        (
            "first_try_attribute_after_nested_branch_aliases.py",
            r"def target(obj, initial, enabled, first, second):
    if first:
        if second:
            obj.note()
    try:
        value = initial
        if obj.enabled and enabled:
            value = obj.other
        obj.consume(value)
    except Error:
        obj.recover()
",
        ),
        (
            "first_try_builtin_after_module_comprehension_restore.py",
            r#"deps = [item for item in items if item]
try:
    if isinstance(value, str) and value == "Auto":
        result = None
    else:
        result = min(int(value), 30)
except Exception:
    result = None
consume(result, deps)
"#,
        ),
        (
            "first_try_method_truth_in_match.py",
            r"async def target(obj, items):
    async for item in items:
        match item:
            case Event():
                note()
                try:
                    if obj.enabled():
                        await act(item)
                    else:
                        await other(item)
                finally:
                    obj.finish(item)
",
        ),
        (
            "first_try_method_empty_class_global_call_control.py",
            r#"async def target(obj, items, stats):
    async for item in items:
        match item:
            case Event():
                note("done")
                try:
                    if obj.enabled():
                        await obj.act(item)
                    else:
                        await obj.other(item)
                finally:
                    obj.finish(item)
            case _:
                obj.ignore(item)
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
