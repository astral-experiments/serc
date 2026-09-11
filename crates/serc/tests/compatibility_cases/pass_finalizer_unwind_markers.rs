use crate::common;

#[test]
fn matches_cpython_pass_finalizer_handler_unwind_owners() {
    for source in [
        r"def collect(value, manager, items, flag):
    try:
        try:
            try:
                prepare(value)
            except Error:
                return value
        finally:
            pass
    finally:
        with manager:
            pass
",
        r"async def collect(value, manager, items, flag):
    yield value
    try:
        try:
            try:
                prepare(value)
            except Error as error:
                observe(error)
                await work(value)
                return
        finally:
            pass
    finally:
        with manager:
            pass
",
    ] {
        common::assert_matches_cpython_marshal(source, "pass_finalizer_handler_unwind_owners.py");
    }
}

#[test]
fn matches_cpython_pass_finalizer_return_markers() {
    for source in [
        r"async def collect(value, manager, items, flag):
    try:
        try:
            return
        finally:
            async with manager:
                pass
    finally:
        pass
",
        r"async def collect(value, manager, items, flag):
    yield value
    try:
        try:
            return
        finally:
            pass
    finally:
        with manager:
            pass
",
    ] {
        common::assert_matches_cpython_marshal(source, "pass_finalizer_return_markers.py");
    }
}

#[test]
fn matches_cpython_pass_finalizer_loop_markers() {
    for source in [
        r"def collect(value, manager, items, flag):
    for item in items:
        try:
            try:
                break
            finally:
                pass
        finally:
            with manager:
                pass
        after_iteration(value)
    after_loop(value)
",
        r"def collect(value, manager, items, flag):
    while flag:
        try:
            try:
                break
            finally:
                pass
        finally:
            with manager:
                pass
        after_iteration(value)
    after_loop(value)
",
        r"async def collect(value, manager, items, flag):
    for item in items:
        try:
            try:
                break
            finally:
                async with manager:
                    pass
        finally:
            pass
        after_iteration(value)
    after_loop(value)
",
        r"async def collect(value, manager, items, flag):
    while flag:
        try:
            try:
                break
            finally:
                async with manager:
                    pass
        finally:
            pass
        after_iteration(value)
    after_loop(value)
",
        r"def collect(value, manager, other, items, flag):
    for item in items:
        try:
            try:
                break
            finally:
                pass
        finally:
            with manager:
                consume(value)
        after_iteration(value)
    after_loop(value)
",
    ] {
        common::assert_matches_cpython_marshal(source, "pass_finalizer_loop_markers.py");
    }
}

#[test]
fn matches_cpython_consecutive_pass_finalizer_join_markers() {
    for source in [
        r"async def collect(value, manager, other, items, flag):
    try:
        try:
            try:
                return value
            finally:
                async with manager:
                    consume(value)
        finally:
            pass
    finally:
        pass
",
        r"async def collect(value, manager, other, items, flag):
    yield value
    try:
        try:
            try:
                return
            finally:
                async with manager:
                    consume(value)
        finally:
            pass
    finally:
        pass
",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "consecutive_pass_finalizer_join_markers.py",
        );
    }
}

#[test]
fn matches_cpython_protected_async_for_finalizer_boundaries() {
    for body in [
        "pass",
        "await consume(item)",
        "yield item",
        "if item:\n    return",
    ] {
        for finalbody in ["pass", "close()", "await close()"] {
            for (prefix, indent, suffix) in [
                ("", "        ", ""),
                ("", "        ", "        after_loop()\n"),
                ("        async for outer in items:\n", "            ", ""),
            ] {
                let body = body.replace('\n', &format!("\n{indent}    "));
                let source = format!(
                    "async def collect(items):\n    try:\n{prefix}{indent}async for item in items:\n{indent}    {body}\n{suffix}    finally:\n        {finalbody}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_async_for_finalizer_boundaries.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_constant_finalizer_unwind_owners() {
    for source in [
        r"def collect(value, manager, outer):
    try:
        try:
            try:
                return value
            finally:
                42
        finally:
            with manager:
                pass
    except Error:
        recover()
",
        r"async def collect(value, manager, outer):
    try:
        try:
            try:
                return work(value)
            finally:
                42
        finally:
            with manager:
                pass
    except Error:
        recover()
",
    ] {
        common::assert_matches_cpython_marshal(source, "constant_finalizer_unwind_owners.py");
    }
}

#[test]
fn matches_cpython_copied_folded_finalizer_tail_owners() {
    for definition in ["def", "async def"] {
        for returned in ["value", "work(value)"] {
            for expression in ["1 + 2", "(1, 2)"] {
                let source = format!(
                    "{definition} collect(value, manager):\n    try:\n        try:\n            try:\n                return {returned}\n            finally:\n                {expression}\n        finally:\n            with manager:\n                pass\n    except Error:\n        recover()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_folded_finalizer_tail_owners.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_first_pass_finalizer_continue_pop_marker() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield items\n"),
        ("async def", "    yield items\n"),
    ] {
        for loop_header in ["for item in items", "while flag"] {
            for test in ["flag", "not flag"] {
                for body in [
                    format!("consume(item)\nif {test}:\n    continue"),
                    format!("with manager:\n    consume(item)\n    if {test}:\n        continue"),
                    format!(
                        "try:\n    prepare(item)\nexcept Error:\n    consume(item)\n    if {test}:\n        continue"
                    ),
                ] {
                    let body = body.replace('\n', "\n            ");
                    let source = format!(
                        "{definition} collect(items, flag, manager):\n    {loop_header}:\n        try:\n            {body}\n        finally:\n            pass\n{footer}    return\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "first_pass_finalizer_continue_pop_marker.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_first_pass_finalizer_break_pop_marker() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield items\n"),
        ("async def", "    yield items\n"),
    ] {
        for test in ["flag", "not flag"] {
            let source = format!(
                "{definition} collect(items, flag, manager):\n    for item in items:\n        try:\n            consume(item)\n            if {test}:\n                break\n        finally:\n            pass\n{footer}    return\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "first_pass_finalizer_break_pop_marker.py",
            );
        }
    }
}

#[test]
fn matches_cpython_constant_try_finally_pop_block_owners() {
    for (source, filename) in [
        ("try:\n    1\nfinally:\n    pass\n", "one_literal.py"),
        (
            "try:\n    1\n    2\nfinally:\n    pass\n",
            "two_literals.py",
        ),
        (
            "try:\n    work()\n    2\nfinally:\n    close()\n",
            "call_then_literal.py",
        ),
        (
            "try:\n    2\n    work()\nfinally:\n    close()\n",
            "literal_then_call.py",
        ),
        (
            "try:\n\x0c    1\n    2\nfinally:\n    pass\n",
            "formfeed_body.py",
        ),
        ("\x0ctry:\n    1\nfinally:\n    pass\n", "formfeed_try.py"),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_constant_try_finally_nested_owner_boundaries() {
    for (source, filename) in [
        (
            "def f(flag):\n    try:\n        if flag:\n            1\n        work()\n    finally:\n        close()\n",
            "nested_branch.py",
        ),
        (
            "def f(items):\n    try:\n        for item in items:\n            1\n        work()\n    finally:\n        close()\n",
            "nested_loop.py",
        ),
        (
            "try:\n    try:\n        1\n    finally:\n        pass\nexcept Error:\n    recover()\n",
            "outer_handler.py",
        ),
        (
            "with manager:\n    try:\n        1\n    finally:\n        pass\n    work()\n",
            "sync_with_following.py",
        ),
        (
            "def f():\n    yield 0\n    try:\n        1\n        2\n    finally:\n        close()\n    yield 3\n",
            "generator.py",
        ),
        (
            "async def f():\n    try:\n        1\n    finally:\n        await close()\n",
            "coroutine.py",
        ),
        (
            "async def f(manager):\n    async with manager:\n        try:\n            1\n        finally:\n            pass\n",
            "async_with.py",
        ),
        (
            "try:\n    try:\n        1\n    finally:\n        pass\nfinally:\n    close()\n",
            "inner_finally.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
