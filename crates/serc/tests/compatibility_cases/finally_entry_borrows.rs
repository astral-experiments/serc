use crate::common;

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
fn matches_cpython_marshal_for_terminal_conditional_finally_loads() {
    let source = "def f(value, flag):\n    try:\n        work()\n    except Error:\n        recover()\n    finally:\n        if flag:\n            consume(value)\n";
    common::assert_matches_cpython_marshal(source, "terminal_conditional_finally_loads.py");
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
fn matches_cpython_raising_else_finally_context_boundaries() {
    for (name, source) in [
        (
            "conditional_raise_handler",
            "def f(x, cond, other, source):\n    try:\n        if cond:\n            work()\n        else:\n            raise Error\n    except Error as error:\n        if other:\n            raise\n    finally:\n        consume(x)\n",
        ),
        (
            "context_outer_try",
            "def f(x, cond, other, source):\n    try:\n        try:\n            if cond:\n                work()\n            else:\n                raise Error\n        except Error:\n            raise\n        finally:\n            consume(x)\n    except BaseException:\n        raise\n",
        ),
        (
            "after_try_work",
            "def f(x, cond, other, source):\n    try:\n        if cond:\n            work()\n        else:\n            raise Error\n    except Error:\n        raise\n    finally:\n        consume(x)\n    consume(other)\n",
        ),
        (
            "context_handler",
            "def f(x, cond, other, source):\n    try:\n        work()\n    except Error:\n        try:\n            if cond:\n                work()\n            else:\n                raise Error\n        except Error:\n            raise\n        finally:\n            consume(x)\n",
        ),
        (
            "protected_prefix_assert",
            "def f(x, cond, other, source):\n    try:\n        assert True\n        if cond:\n            work()\n        else:\n            raise Error\n    except Error:\n        raise\n    finally:\n        consume(x)\n",
        ),
        (
            "constant_true_test",
            "def f(x, cond, other, source):\n    try:\n        if True:\n            work()\n        else:\n            raise Error\n    except Error:\n        raise\n    finally:\n        consume(x)\n",
        ),
        (
            "constant_false_test",
            "def f(x, cond, other, source):\n    try:\n        if False:\n            work()\n        else:\n            raise Error\n    except Error:\n        raise\n    finally:\n        consume(x)\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, &format!("raising_else_{name}.py"));
    }
}
