use crate::common;

#[test]
fn matches_cpython_protected_loop_condition_exit_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for loop_header in ["for item in items:", "while active:"] {
            for condition in [
                "a or b",
                "not (a and b)",
                "(a and b) or c",
                "a and b",
                "False or a",
                "a < b < c",
            ] {
                for name in ["", " as error"] {
                    let source = format!(
                        "{function} f(items, active, a, b, c):\n{suspension}    {loop_header}\n        try:\n            if {condition}:\n                work()\n        except Exception{name}:\n            recover()\n    after()\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "protected_loop_condition_exit_locations.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_finally_backedges_after_returning_try_handlers() {
    for (function, suspension) in [("def", ""), ("async def", ""), ("def", "    yield seed\n")] {
        for loop_header in ["for item in items:", "while flag:"] {
            for name in ["", " as error"] {
                let source = format!(
                    "{function} f(items, flag, other, seed, manager):\n{suspension}    {loop_header}\n        try:\n            return seed\n        except Error{name}:\n            recover(seed)\n        finally:\n            if flag:\n                cleanup(seed)\n    done(seed)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_condition_exit_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_finally_backedges_after_structured_cleanup() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield seed\n"),
        ("async def", "    yield seed\n"),
    ] {
        for loop_header in ["for item in items:", "while flag:"] {
            for finalizer in [
                "            prepare(seed)\n            if flag:\n                cleanup(seed)\n",
                "            if flag:\n                cleanup(seed)\n                finish(seed)\n",
                "            if flag:\n                if other:\n                    cleanup(seed)\n",
            ] {
                let source = format!(
                    "{function} f(items, flag, other, seed, manager):\n{suspension}    {loop_header}\n        try:\n            work(seed)\n        finally:\n{finalizer}    done(seed)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_condition_exit_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_finally_manager_cleanup_joins() {
    for (function, suspension, manager_kind) in [
        ("def", "", "with"),
        ("async def", "", "async with"),
        ("def", "    yield seed\n", "with"),
        ("async def", "    yield seed\n", "async with"),
    ] {
        for (before, after) in [
            ("", ""),
            ("                prepare(seed)\n", ""),
            ("", "                finish(seed)\n"),
        ] {
            let source = format!(
                "{function} f(items, flag, other, seed, manager):\n{suspension}    for item in items:\n        try:\n            work(seed)\n        finally:\n            if flag:\n{before}                {manager_kind} manager:\n                    cleanup(seed)\n{after}    done(seed)\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_loop_condition_exit_locations.py",
            );
        }
    }
}

#[test]
fn matches_cpython_try_else_backedge_reachability() {
    for function in ["def", "async def"] {
        for loop_header in ["for item in items:", "while flag:"] {
            for try_body in ["return seed", "work(seed)"] {
                let source = format!(
                    "{function} f(items, flag, other, seed, manager):\n    {loop_header}\n        try:\n            {try_body}\n        except Error:\n            recover(seed)\n        else:\n            if flag:\n                cleanup(seed)\n    done(seed)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_condition_exit_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_for_boolean_discard_locations_in_suspended_functions() {
    for (function, suspension) in [("async def", ""), ("def", "    yield None\n")] {
        for body in [
            "        (\n            left(item)\n            and right(item)\n        )\n",
            "        first(item) or second(item) or third(item)\n",
            "        left(item) and (middle(item) or right(item))\n",
        ] {
            let source = format!(
                "{function} f(items, flag, item):\n{suspension}    for item in items:\n{body}    done()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_loop_condition_exit_locations.py",
            );
        }
    }

    common::assert_matches_cpython_marshal(
        "async def f(items, flag, item):\n    yield None\n    for item in items:\n        (left(item)\n         and right(item))\n    done()\n",
        "protected_loop_condition_exit_locations.py",
    );
}

#[test]
fn matches_cpython_loop_boolean_discard_boundaries() {
    for (function, suspension, loop_header) in [
        ("def", "", "while flag:"),
        ("def", "", "while True:"),
        ("async def", "", "async for item in items:"),
        ("def", "    yield None\n", "while True:"),
    ] {
        for body in [
            "        left(item) and right(item)\n        consume(item)\n",
            "        result = left(item) and right(item)\n",
        ] {
            let source = format!(
                "{function} f(items, flag, item):\n{suspension}    {loop_header}\n{body}    done()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "protected_loop_condition_exit_locations.py",
            );
        }
    }
}

#[test]
fn matches_cpython_folded_loop_boolean_discard_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield None\n"),
        ("async def", "    yield None\n"),
    ] {
        let source = format!(
            "{function} f(items):\n{suspension}    sentinel = False\n    for item in items:\n        (False or True) and (True or False)\n    done()\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "protected_loop_condition_exit_locations.py",
        );
    }
}

#[test]
fn matches_cpython_same_operator_constant_loop_boolean_discard_locations() {
    for expression in [
        "left(item) and (True and False)",
        "left(item) or (False or True)",
    ] {
        let source =
            format!("def f(items):\n    for item in items:\n        {expression}\n    done()\n");
        common::assert_matches_cpython_marshal(
            &source,
            "protected_loop_condition_exit_locations.py",
        );
    }
}
