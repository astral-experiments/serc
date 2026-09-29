use crate::common;

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
