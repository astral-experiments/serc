use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_module_with_discarded_tail_boundaries() {
    for (filename, source) in [
        (
            "with_then_constant_terminal_if.py",
            "if enabled:\n    with manager:\n        use()\n    \"ignored\"\n",
        ),
        (
            "with_then_false_terminal_if.py",
            "with manager:\n    use()\nif False:\n    ignored()\n",
        ),
        (
            "terminal_with_control.py",
            "if enabled:\n    with manager:\n        use()\n",
        ),
        (
            "with_then_call_control.py",
            "with manager:\n    use()\nfinish()\n",
        ),
        (
            "module_with_string_single.py",
            "with manager:\n    use()\n\"ignored\"\n",
        ),
        (
            "module_with_string_multiline.py",
            "with manager:\n    use()\n\"\"\"ignored\nsecond line\n\"\"\"\n",
        ),
        (
            "module_nested_with_string_multiline.py",
            "with first:\n    with second:\n        use()\n\"\"\"ignored\nsecond line\n\"\"\"\n",
        ),
        (
            "folded_true_with_string_control.py",
            "if True:\n    with manager:\n        use()\n    \"ignored\"\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_terminal_named_with_cleanup_copies() {
    for (scope, prefix, with_keyword) in [
        ("def", "", "with"),
        ("async def", "", "with"),
        ("async def", "", "async with"),
        ("def", "    yield seed\n", "with"),
    ] {
        for handler in ["except Error as error", "except Error", "except"] {
            for body in ["recover()", "recovered = recover()"] {
                for successor in ["", "    done()\n"] {
                    let source = format!(
                        "{scope} f(manager, seed):\n{prefix}    try:\n        work()\n    {handler}:\n        {with_keyword} manager:\n            {body}\n{successor}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_named_with_cleanup.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_named_handler_cleanup_after_suppressed_with() {
    for outer in [false, true] {
        for work in ["work()", "with manager:\n    work()"] {
            for preceding_handler in ["", "except Other:\n    report()\n"] {
                for prefix in ["", "            report()\n"] {
                    for returned in ["None", "flag"] {
                        let handler = format!(
                            "try:\n{}{preceding_handler}except Error as error:\n    if flag:\n        with manager:\n{prefix}            raise error\n    else:\n        raise error\n",
                            indent(work)
                        );
                        let body = if outer {
                            format!("if enabled:\n{}", indent(&handler))
                        } else {
                            handler
                        };
                        let source = format!(
                            "def f(flag, enabled, manager):\n{}    return {returned}\n",
                            indent(&body)
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "named_suppressed_with_cleanup.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_cleanup_and_return_owners() {
    for early_return in ["", "    if enabled:\n        return False\n"] {
        for prefix in ["", "        value = compute()\n"] {
            for alias in ["", " as response"] {
                for chain in [false, true] {
                    let returned = if chain {
                        if alias.is_empty() {
                            "0 <= value < 3"
                        } else {
                            "0 <= response.status < 3"
                        }
                    } else {
                        "True"
                    };
                    let source = format!(
                        "def f(manager, value, enabled):\n{early_return}    try:\n        work()\n    except Error:\n{prefix}        try:\n            with manager{alias}:\n                return {returned}\n        except Other:\n            return False\n    return True\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_handler_cleanup.py");
                }
            }
        }
    }
}

#[test]
fn preserves_implicit_returns_after_suppressed_with_handlers() {
    for outer in [false, true] {
        for body in [
            "with manager:\n            raise error",
            "with manager: raise error",
        ] {
            let handler = format!(
                "try:\n    work()\nexcept Error as error:\n    if flag:\n        {body}\n    else:\n        raise error\n"
            );
            let suite = if outer {
                format!("if enabled:\n{}", indent(&handler))
            } else {
                handler
            };
            let source = format!("def f(flag, enabled, manager):\n{}", indent(&suite));
            common::assert_matches_cpython_marshal(&source, "implicit_with_handler_return.py");
        }
    }
}

#[test]
fn matches_cpython_nested_handler_return_owners_after_suppressed_with() {
    for scope in ["def", "async def"] {
        for returned in ["False", "value", "build()"] {
            for prefix in ["", "        value = compute()\n"] {
                for alias in ["", " as response"] {
                    let source = format!(
                        "{scope} f(manager, value):\n    try:\n        work()\n    except Error:\n{prefix}        try:\n            with manager{alias}:\n                return True\n        except Other:\n            return {returned}\n    return True\n",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_handler_suppressed_with_owner.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_pop_preserves_intervening_manager() {
    for returned in ["False", "value"] {
        let source = format!(
            "def process(manager, surrounding, value):\n    try:\n        work()\n    except Error:\n        with surrounding:\n            try:\n                with manager:\n                    return True\n            except Other:\n                return {returned}\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_handler_intervening_manager.py");
    }
}

#[test]
fn matches_cpython_original_manager_pop_blocks_after_finally() {
    common::assert_matches_cpython_marshal(
        r"async def f(manager, items):
    count = 0
    with manager:
        try:
            async for item in items:
                count += 1
                yield item
        finally:
            report(count)
",
        "1814_async_for_finally.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(manager, first, second):
    with manager:
        try:
            prepare()
            with first, second:
                yield
        finally:
            try:
                flush()
                flush_other()
            except Error:
                pass
            try:
                restore()
            finally:
                close()
",
        "1688_nested_finally.py",
    );
    common::assert_matches_cpython_marshal(
        r"def f(manager):
    with manager:
        try:
            work()
        finally:
            close()
",
        "manager_pop_simple_control.py",
    );
}

#[test]
fn nested_manager_reraise_matches_cpython_handler_cleanup() {
    for (scope, suspension) in [
        ("def", ""),
        ("def", "    yield value\n"),
        ("async def", ""),
        ("async def", "    await before()\n"),
    ] {
        for outer in ["except Error as error:", "except Error:"] {
            for inner in ["except Error:", "except Error as inner:"] {
                for outer_body in ["return run()", "run()"] {
                    for inner_body in ["return retry()", "retry()"] {
                        for manager_body in ["raise", "raise Error", "work()\nraise"] {
                            let manager_body = manager_body.replace('\n', "\n                ");
                            let source = format!(
                                "{scope} f():\n{suspension}    try:\n        {outer_body}\n    {outer}\n        try:\n            {inner_body}\n        {inner}\n            with context():\n                {manager_body}\n"
                            );
                            common::assert_matches_cpython_marshal(
                                &source,
                                "nested_manager_reraise.py",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_manager_fallthrough_matches_cpython_handler_cleanup() {
    for (scope, suspension) in [
        ("def", ""),
        ("def", "    yield value\n"),
        ("async def", ""),
        ("async def", "    await before()\n"),
    ] {
        for outer in ["except Error as error:", "except Error:"] {
            for inner in ["except Error:", "except Error as inner:"] {
                for outer_body in ["return run()", "run()"] {
                    for inner_body in ["return retry()", "retry()"] {
                        let source = format!(
                            "{scope} f():\n{suspension}    try:\n        {outer_body}\n    {outer}\n        try:\n            {inner_body}\n        {inner}\n            with context():\n                work()\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "nested_manager_fallthrough.py",
                        );
                    }
                }
            }
        }
    }
}
