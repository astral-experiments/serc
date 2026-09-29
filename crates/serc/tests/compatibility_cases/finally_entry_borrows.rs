use crate::common;

#[test]
fn coroutine_guarded_pass_recovery_preserves_owned_finalizer_loads() {
    for guard in ["x and y", "x and y and z"] {
        for inner in ["except E:", "except E as error:"] {
            for outer in ["except F:", "except F as error:"] {
                for body in ["pass", "work(y)"] {
                    for cleanup in [
                        "use(x)",
                        "data = [x, y]\n        use(data)",
                        "data = [x, 1 if y else 2]\n        use(data)",
                    ] {
                        let source = format!(
                            "async def f(x, y, z):\n    try:\n        if {guard}:\n            try:\n                pass\n            {inner}\n                pass\n    {outer}\n        pass\n    try:\n        {body}\n    finally:\n        {cleanup}\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "coroutine_guarded_pass_recovery_finally.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_guarded_pass_recovery() {
    for guard in ["x", "x and y", "x or y", "x is None"] {
        for body in ["pass", "work(y)"] {
            for name in ["", " as error"] {
                for recovery in ["pass", "work(x)"] {
                    for tail in [
                        "    return use(x)\n",
                        "    try:\n        pass\n    finally:\n        data = [x, 1 if y is None else 2]\n        try:\n            consume(data)\n        except E:\n            pass\n",
                    ] {
                        let source = format!(
                            "def f(x, y):\n    try:\n        if {guard}:\n            try:\n                {body}\n            except E{name}:\n                {recovery}\n    except E:\n        pass\n{tail}"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "guarded_pass_recovery_borrows.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_in_structured_finally_bodies() {
    for prefix in ["", "async "] {
        for body in ["use(value)", "for item in values:\n            use(item)"] {
            for handler in ["pass", "use(value)"] {
                for cleanup in [
                    "if value:\n            use(other)",
                    "for item in values:\n            use(value)",
                ] {
                    for successor in ["", "    use(value)\n"] {
                        let source = format!(
                            "{prefix}def f(value, other, values):\n    try:\n        {body}\n    except Error:\n        {handler}\n    finally:\n        {cleanup}\n{successor}"
                        );
                        common::assert_matches_cpython_marshal(&source, "finally_entry_borrows.py");
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_at_nested_try_finally_entry() {
    for prefix in ["", "async "] {
        for body in ["use(value)", "value = produce(value)"] {
            for handler in ["pass", "recover(value)"] {
                for cleanup in ["use(value)", "value = produce(value)"] {
                    for successor in ["", "    return value, other\n"] {
                        let source = format!(
                            "{prefix}def f(value, other):\n    try:\n        {body}\n    except Error:\n        {handler}\n    finally:\n        try:\n            {cleanup}\n        except Error:\n            pass\n{successor}"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "nested_try_finally_entry.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_owned_loads_after_nested_handler_finally_joins() {
    for prefix in ["", "async "] {
        for successor in ["", "    return value\n"] {
            let source = format!(
                "{prefix}def f(value):\n    try:\n        use(value)\n    except Error as error:\n        try:\n            use(value)\n        except Error:\n            pass\n    finally:\n        try:\n            use(value)\n        except Error:\n            pass\n{successor}"
            );
            common::assert_matches_cpython_marshal(&source, "nested_handler_finally_join.py");
        }
    }
}

#[test]
fn matches_cpython_finally_borrows_after_empty_handler_and_else_exits() {
    for (prefix, work) in [("", "work()"), ("async ", "await work()")] {
        for handler in ["recover(value)", "with value:\n            recover(value)"] {
            for otherwise in [
                "",
                "    else:\n        if flag:\n            process(value)\n",
                "    else:\n        if flag:\n            process(value)\n        else:\n            process(value)\n",
                "    else:\n        if flag:\n            with value:\n                process(value)\n",
                "    else:\n        with value:\n            process(value)\n",
            ] {
                for cleanup in ["finish(value)", "if flag:\n            finish(value)"] {
                    let source = format!(
                        "{prefix}def f(value, flag):\n    try:\n        {work}\n    except Error as error:\n        {handler}\n{otherwise}    finally:\n        {cleanup}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "empty_finally_entry_borrows.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_direct_finally_entry_across_handler_kinds() {
    for handler in [
        "except Error:\n    pass",
        "except Error as error:\n    pass",
        "except Error as error:\n    recover(error)",
        "except Error:\n    raise",
        "except:\n    pass",
        "except Error as error:\n    if flag:\n        raise",
    ] {
        for cleanup in [
            "cleanup(value)",
            "obj.attr = value",
            "if value:\n    cleanup(value)",
            "if value:\n    cleanup(value)\nelse:\n    cleanup(other)",
        ] {
            let source = format!(
                "def f(value, flag, other, obj):\n    try:\n        if flag:\n            work(value)\n        else:\n            work(other)\n    {}\n    finally:\n        {}\n    consume(value)\n",
                handler.replace('\n', "\n    "),
                cleanup.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "direct_finally_handler_entry.py");
        }
    }
}

#[test]
fn matches_cpython_owned_finally_loads_after_empty_conditional_arms() {
    for body in [
        "if flag:\n    work(value)",
        "if flag:\n    assert True\nelse:\n    work(other)",
    ] {
        for cleanup in [
            "cleanup(value)",
            "try:\n    cleanup(value)\nfinally:\n    work(other)",
        ] {
            let source = format!(
                "def f(value, flag, other):\n    try:\n        {}\n    except Error:\n        raise\n    finally:\n        {}\n    consume(value)\n",
                body.replace('\n', "\n        "),
                cleanup.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "empty_conditional_finally_entry.py");
        }
    }
}

#[test]
fn matches_cpython_direct_finally_entry_with_conditional_assignment_targets() {
    for handler in [
        "except Error:\n    pass",
        "except Error as error:\n    recover(error)",
        "except Error:\n    raise",
        "except:\n    pass",
    ] {
        for cleanup in [
            "target[value if flag else other] = value",
            "target[value if flag else other] += value",
            "(value if flag else other).field = value",
            "(value if flag else other).field += value",
            "target[value if flag else other]: Kind = value",
            "target[value and other] = value",
        ] {
            let source = format!(
                "def f(value, flag, other, target):\n    try:\n        if flag:\n            work(value)\n        else:\n            work(other)\n    {}\n    finally:\n        {cleanup}\n    consume(value)\n",
                handler.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(
                &source,
                "conditional_finally_assignment_targets.py",
            );
        }
    }
}

#[test]
fn matches_cpython_direct_finally_entry_with_suspending_cleanup() {
    for (prefix, expression) in [
        ("async ", "await work(value)"),
        ("", "yield value"),
        ("", "yield from value"),
    ] {
        for handler in [
            "except Error:\n    pass",
            "except Error as error:\n    recover(error)",
            "except:\n    pass",
        ] {
            for conditional in [false, true] {
                let cleanup = if conditional {
                    format!(
                        "if flag:\n    {expression}\nelse:\n    {}",
                        expression.replace("value", "other")
                    )
                } else {
                    expression.to_owned()
                };
                let source = format!(
                    "{prefix}def f(value, flag, other):\n    try:\n        if flag:\n            work(value)\n        else:\n            work(other)\n    {}\n    finally:\n        {}\n    consume(value)\n",
                    handler.replace('\n', "\n    "),
                    cleanup.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "suspending_finally_entry.py");
            }
        }
    }
}

fn assert_raising_else_finally_matches_cpython(
    name: &str,
    body: &str,
    handler: &str,
    cleanup: &str,
) {
    let source = format!(
        "def f(x, cond, other, source):\n    try:\n        {}\n    {}\n    finally:\n        {}\n",
        body.replace('\n', "\n        "),
        handler.replace('\n', "\n    "),
        cleanup.replace('\n', "\n        "),
    );
    common::assert_matches_cpython_marshal(&source, &format!("raising_else_{name}.py"));
}

#[test]
fn matches_cpython_finally_entry_after_raising_else_arms() {
    for (name, body) in [
        (
            "base_raise_else",
            "if cond:\n    work()\nelse:\n    raise Error",
        ),
        (
            "live_elif_between_raises",
            "if cond:\n    raise Error\nelif other:\n    work()\nelse:\n    raise Error",
        ),
        ("bare_raise_else", "if cond:\n    work()\nelse:\n    raise"),
        (
            "cause_raise_else",
            "if cond:\n    work()\nelse:\n    raise Error(x) from None",
        ),
        (
            "prefix_raise_else",
            "if cond:\n    work()\nelse:\n    log(x)\n    raise Error",
        ),
    ] {
        assert_raising_else_finally_matches_cpython(
            name,
            body,
            "except Error:\n    raise",
            "consume(x)",
        );
    }
}

#[test]
fn matches_cpython_raising_else_with_conditional_cleanup() {
    for (name, cleanup) in [
        (
            "conditional_cleanup",
            "if predicate(x):\n    consume(other)",
        ),
        (
            "conditional_else_cleanup",
            "if predicate(x):\n    consume(other)\nelse:\n    consume(source)",
        ),
    ] {
        assert_raising_else_finally_matches_cpython(
            name,
            "if cond:\n    work()\nelse:\n    raise Error",
            "except Error:\n    raise",
            cleanup,
        );
    }
}

#[test]
fn matches_cpython_finally_calls_reached_from_nested_arms() {
    for (prefix, cleanup) in [("", "x.close()"), ("async ", "await x.close()")] {
        for (name, body) in [
            (
                "nested_then",
                "if pred:\n    if cond:\n        x.work()\nelse:\n    x.wait()",
            ),
            (
                "inner_try",
                "if pred:\n    try:\n        x.work()\n    except E:\n        pass\nelse:\n    x.wait()",
            ),
            (
                "conditional_return",
                "if pred:\n    if cond:\n        return\n    x.work()\nelse:\n    x.wait()",
            ),
            (
                "folded_assert",
                "if pred:\n    if cond:\n        assert True\n    else:\n        x.work()\nelse:\n    x.wait()",
            ),
        ] {
            let source = format!(
                "{prefix}def f(x, pred, cond):\n    try:\n        {}\n    except E:\n        pass\n    finally:\n        {cleanup}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("nested_finally_call_{name}.py"),
            );
        }
    }
}

#[test]
fn preserves_cpython_finally_entries_after_conditional_and_empty_paths() {
    for (prefix, cleanup) in [("", "x.close()"), ("async ", "await x.close()")] {
        for (name, body) in [
            ("plain_if", "if pred:\n    x.work()"),
            ("no_outer_else", "if pred:\n    if cond:\n        x.work()"),
            (
                "nested_else",
                "if pred:\n    x.work()\nelse:\n    if cond:\n        x.wait()",
            ),
        ] {
            let source = format!(
                "{prefix}def f(x, pred, cond):\n    try:\n        {}\n    except E:\n        pass\n    finally:\n        {cleanup}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("conditional_finally_call_{name}.py"),
            );
        }
    }
}

#[test]
fn preserves_owned_finally_loads_after_removed_zero_distance_jumps() {
    // Inspect these functions without calling their bodies. An additional call after
    // the nested with keeps the native predecessor nonempty and allows borrowing.
    for (source, expected) in [
        (
            r"def probe(active, outer, inner, manager, touch, report, pause):
    while active():
        try:
            if outer():
                if inner():
                    with manager():
                        touch()
        except Exception:
            report()
        finally:
            pause()
",
            "LOAD_FAST LOAD_FAST\n",
        ),
        (
            r"def probe(active, outer, inner, manager, touch, report, pause):
    while active():
        try:
            if outer():
                if inner():
                    with manager():
                        touch()
            touch()
        except Exception:
            report()
        finally:
            pause()
",
            "LOAD_FAST_BORROW LOAD_FAST\n",
        ),
        (
            r"def probe(active, outer, inner, manager, touch, report, pause):
    while active():
        try:
            with manager():
                touch()
        except Exception:
            report()
        finally:
            pause()
",
            "LOAD_FAST LOAD_FAST\n",
        ),
    ] {
        let source = format!(
            r#"import dis
{source}
print(" ".join(
    instruction.opname
    for instruction in dis.get_instructions(probe)
    if instruction.argval == "pause" and instruction.opname.startswith("LOAD_FAST")
))
"#
        );
        common::assert_executes(&source, expected);
    }
}

#[test]
fn matches_cpython_finally_borrows_after_try_star_original_empty_exits() {
    for body in [
        "work(value)",
        "with manager:\n            work(value)",
        "async with manager:\n            await work(value)",
    ] {
        for successor in [
            "consume(value)\n    consume(other)",
            "return value",
            "return None",
        ] {
            let source = format!(
                "async def f(value, other, manager):\n    try:\n        {body}\n    except* Error:\n        recover(other)\n    {successor}\n"
            );
            common::assert_matches_cpython_marshal(&source, "try_star_normal_exit_borrows.py");
        }
        for orelse in ["", "    else:\n        consume(value)\n"] {
            let source = format!(
                "async def f(value, other, manager):\n    try:\n        {body}\n    except* Error:\n        recover(other)\n{orelse}    if value:\n        consume(other)\n    consume(value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "try_star_else_exit_borrows.py");
        }
        let nested_body = body.replace('\n', "\n    ");
        let source = format!(
            "async def f(value, other, manager, flag):\n    try:\n        try:\n            {nested_body}\n        except* Error:\n            recover(other)\n        if flag:\n            work(value)\n        elif other:\n            work(other)\n    except Error:\n        raise\n    finally:\n        if value:\n            consume(value)\n        consume(other)\n"
        );
        common::assert_matches_cpython_marshal(&source, "try_star_outer_finally_borrows.py");
    }
}

#[test]
fn matches_cpython_finally_borrows_after_authored_loop_breaks() {
    for (prefix, suspend) in [("", ""), ("async ", "await ")] {
        let source = format!(
            r#"{prefix}def probe(resource, stream, active):
    try:
        prepare()
        while True:
            try:
                {suspend}stream.read()
            except Exception as error:
                report(error)
            if active():
                break
            {suspend}stream.write()
    except Exception as error:
        report(error)
    finally:
        report("closing")
        {suspend}resource.close()
"#
        );
        common::assert_matches_cpython_marshal(&source, "authored_break_finally_borrows.py");
    }
}

#[test]
fn matches_cpython_conditional_body_while_finalizer_borrows() {
    for body in [
        "work(value)",
        "if flag:\n            work(value)",
        "if flag:\n            work(value)\n        else:\n            other(value)",
    ] {
        let source = format!(
            "def f(value, flag):
    try:
        {body}
    finally:
        while value.pending:
            item = value.pending.pop()
            close(item)
    report(value)
"
        );
        common::assert_matches_cpython_marshal(&source, "conditional_body_while_finalizer.py");
    }
}

#[test]
fn matches_cpython_nested_conditional_named_recovery_finalizer_borrows() {
    for prefix in ["", "prepare(value)\n            "] {
        for recovery in ["a, b, c = error.args", "report(error)", "value = error"] {
            let source = format!(
                "def f(value, flag):
    try:
        value.state = flag
        try:
            {prefix}if flag:
                work(value)
        except Exception as error:
            {recovery}
    finally:
        close(value)
    report(value)
"
            );
            common::assert_matches_cpython_marshal(&source, "nested_named_recovery_finalizer.py");
        }
    }
}

#[test]
fn matches_cpython_iterator_continue_recovery_finally_borrows() {
    for (managed, guarded_prefix) in [(false, false), (false, true), (true, false), (true, true)] {
        let prefix = if guarded_prefix {
            "            if report:\n                obj.report(item)\n"
        } else {
            ""
        };
        let body = format!(
            "        try:\n            value = obj.load(item)\n        except ValueError:\n{prefix}            if retry:\n                continue\n            obj.skip(item)\n        finally:\n            status = obj.finish(item)\n        obj.check(value, status)\n"
        );
        let body = if managed {
            let mut indented = String::new();
            for line in body.lines() {
                indented.push_str("    ");
                indented.push_str(line);
                indented.push('\n');
            }
            format!("        with obj.manager(item):\n{indented}")
        } else {
            body
        };
        let source =
            format!("def recover(items, obj, report, retry):\n    for item in items:\n{body}");
        common::assert_matches_cpython_marshal(
            &source,
            &format!("iterator_continue_finally_{managed}_{guarded_prefix}.py"),
        );
    }
}
