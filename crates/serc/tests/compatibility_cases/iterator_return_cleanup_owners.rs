use crate::common;

#[test]
fn manager_returns_keep_only_protected_iterator_cleanup_owned() {
    for prefix in ["", "async "] {
        for value in ["None", "value"] {
            for body in [
                "try:\n    for value in values:\n        with manager:\n            prepare()\n            if flag:\n                return VALUE\n            use()\nexcept Error:\n    recover()",
                "for value in values:\n    try:\n        with manager:\n            prepare()\n            if flag:\n                return VALUE\n            use()\n    except Error:\n        recover()",
            ] {
                let source = format!(
                    "{prefix}def f(values, flag):\n    {}\n",
                    body.replace("VALUE", value).replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "manager_iterator_return_owner.py");
            }
        }
    }
}

#[test]
fn literal_returns_keep_iterator_cleanup_inside_the_enclosing_try() {
    for prefix in ["", "async "] {
        for value in ["", " None", " True", " 0"] {
            for preceding in ["", "            prepare()\n"] {
                let source = format!(
                    "{prefix}def f(values, flag):\n    try:\n        for value in values:\n{preceding}            if flag:\n                return{value}\n    except Error:\n        recover()\n"
                );
                common::assert_matches_cpython_marshal(&source, "literal_iterator_return.py");
            }
        }
    }
}

#[test]
fn returns_end_inner_try_before_manager_owned_iterator_cleanup() {
    for (prefix, manager, iterator) in [
        ("", "with", "for"),
        ("async ", "with", "for"),
        ("async ", "async with", "for"),
        ("async ", "with", "async for"),
        ("async ", "async with", "async for"),
    ] {
        for value in ["", " None", " value", " call(value)"] {
            let source = format!(
                "{prefix}def f(manager, items, value):\n    {manager} manager:\n        {iterator} item in items:\n            try:\n                return{value}\n            except Error:\n                pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "return_iterator_try_owner.py");
        }
    }
}

#[test]
fn nested_iterator_returns_end_each_try_at_its_own_cleanup() {
    for (prefix, manager) in [("", "with"), ("async ", "async with")] {
        for value in ["", " value"] {
            let source = format!(
                "{prefix}def f(manager, items, value):\n    {manager} manager:\n        for outer in items:\n            try:\n                for inner in items:\n                    try:\n                        return{value}\n                    except Error:\n                        pass\n            except OuterError:\n                pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_iterator_try_owners.py");
        }
    }
}

#[test]
fn inner_manager_return_cleanup_precedes_outer_iterator_cleanup() {
    for (prefix, manager) in [("", "with"), ("async ", "async with")] {
        for value in ["", " value"] {
            let source = format!(
                "{prefix}def f(manager, other, items, value):\n    {manager} manager:\n        for item in items:\n            {manager} other:\n                try:\n                    return{value}\n                except Error:\n                    pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "manager_before_iterator_unwind.py");
        }
    }
}

#[test]
fn conditional_returns_unwind_iterators_and_tries_in_nesting_order() {
    for (prefix, iterator) in [("", "for"), ("async ", "async for")] {
        for value in [
            "x if flag else y",
            "x if flag else None",
            "None if flag else y",
            "x if flag else y if other else None",
            "(x if flag else y) if other else z",
            "call(x) if flag else call(y)",
        ] {
            for body in [
                "try:\n    ITERATOR item in items:\n        return VALUE\nexcept ValueError:\n    consume(x)",
                "ITERATOR item in items:\n    try:\n        return VALUE\n    except ValueError:\n        consume(x)",
                "try:\n    ITERATOR item in items:\n        try:\n            return VALUE\n        except ValueError:\n            consume(x)\nexcept ValueError:\n    consume(x)",
                "ITERATOR item in items:\n    try:\n        ITERATOR inner in items:\n            return VALUE\n    except ValueError:\n        consume(x)",
            ] {
                let source = format!(
                    "{prefix}def f(items, x, y, z, flag, other):\n    {}\n    return None\n",
                    body.replace("ITERATOR", iterator)
                        .replace("VALUE", value)
                        .replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "conditional_iterator_return.py");
            }
        }
    }
}

#[test]
fn pending_finally_iterator_cleanup_ends_only_inner_tries() {
    for (prefix, iterator, generator) in [
        ("", "for", ""),
        ("async ", "for", ""),
        ("async ", "async for", ""),
        ("", "for", "    yield None\n"),
    ] {
        for value in ["x", "call(x)", "None", "x if flag else y"] {
            for body in [
                "try:\n    ITERATOR item in items:\n        try:\n            return VALUE\n        except ValueError:\n            consume(x)\nfinally:\n    consume(x)",
                "ITERATOR item in items:\n    try:\n        try:\n            return VALUE\n        except ValueError:\n            consume(x)\n    finally:\n        consume(x)",
                "try:\n    try:\n        ITERATOR item in items:\n            try:\n                return VALUE\n            except ValueError:\n                consume(x)\n    finally:\n        consume(x)\nfinally:\n    consume(x)",
            ] {
                let value = if !prefix.is_empty() && value == "call(x)" {
                    "await call(x)"
                } else {
                    value
                };
                let source = format!(
                    "{prefix}def f(items, x, y, flag, other, manager):\n{generator}    {}\n    return None\n",
                    body.replace("ITERATOR", iterator)
                        .replace("VALUE", value)
                        .replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "finally_iterator_return.py");
            }
        }
    }
}

#[test]
fn handler_returns_keep_iterator_cleanup_in_pending_finally() {
    for (prefix, iterator, prologue, call) in [
        ("", "for", "", "call(x)"),
        ("async ", "for", "", "await call(x)"),
        ("async ", "async for", "", "await call(x)"),
        ("", "for", "    yield None\n", "call(x)"),
    ] {
        for handler_name in ["", " as error"] {
            for value in ["None", "x", call, "x if flag else y"] {
                for nested in [false, true] {
                    let mut body = format!(
                        "try:\n    consume(x)\nexcept ValueError{handler_name}:\n    return {value}",
                    );
                    for _ in 0..if nested { 2 } else { 1 } {
                        body = format!(
                            "{iterator} item in items:\n    {}",
                            body.replace('\n', "\n    "),
                        );
                    }
                    let source = format!(
                        "{prefix}def f(items, x, y, flag):\n{prologue}    try:\n        {}\n    finally:\n        consume(x)\n    return None\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "handler_return_finally_iterator_owner.py",
                    );
                }
            }
        }
    }
}

#[test]
fn handler_returns_keep_outer_try_owner_through_iterators() {
    for (prefix, iterator, prologue, call) in [
        ("", "for", "", "call(x)"),
        ("async ", "for", "", "await call(x)"),
        ("async ", "async for", "", "await call(x)"),
        ("", "for", "    yield None\n", "call(x)"),
    ] {
        for handler_name in ["", " as error"] {
            for value in ["None", "x", call, "x if flag else y"] {
                for nested in [false, true] {
                    if nested && iterator == "async for" {
                        continue;
                    }
                    let mut body = format!(
                        "try:\n    consume(x)\nexcept ValueError{handler_name}:\n    return {value}",
                    );
                    for _ in 0..if nested { 2 } else { 1 } {
                        body = format!(
                            "{iterator} item in items:\n    {}",
                            body.replace('\n', "\n    "),
                        );
                    }
                    let source = format!(
                        "{prefix}def f(items, x, y, flag):\n{prologue}    try:\n        try:\n            {}\n        except TypeError:\n            consume(y)\n    finally:\n        consume(x)\n    return None\n",
                        body.replace('\n', "\n            "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "handler_return_outer_try_iterator_owner.py",
                    );
                }
            }
        }
    }
}

#[test]
fn call_finally_preserves_inner_iterator_cleanup_owner() {
    for source in [
        "def f(items, value):\n    try:\n        for item in items:\n            return value\n    except ValueError:\n        recover()\n    finally:\n        cleanup()\n",
        "async def f(items, value, manager):\n    try:\n        async with manager:\n            for item in items:\n                try:\n                    return value\n                except ValueError:\n                    recover()\n    finally:\n        cleanup()\n",
        "def f(items, value):\n    try:\n        for item in items:\n            try:\n                return value\n            except ValueError:\n                recover()\n    finally:\n        cleanup()\n",
    ] {
        common::assert_matches_cpython_marshal(source, "call_finally_iterator_owner.py");
    }
}

#[test]
fn suppressed_manager_return_keeps_try_owner_before_following_break() {
    for (filename, source) in [
        (
            "manager_suppression_following_break.py",
            r"def target(items, manager, work, report):
    for item in items:
        try:
            with manager as value:
                result = work(value)
                return result
            break
        except ValueError as error:
            report(error)
    return None
",
        ),
        (
            "manager_suppression_loop_backedge_control.py",
            r"def target(items, manager, work, report):
    for item in items:
        try:
            with manager as value:
                result = work(value)
                return result
        except ValueError as error:
            report(error)
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
