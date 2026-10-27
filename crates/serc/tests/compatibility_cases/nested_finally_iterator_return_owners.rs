use crate::common;

#[test]
fn matches_cpython_nested_finally_iterator_return_owners() {
    for (definition, suspension) in [
        ("def", ""),
        ("def", "    yield 0\n"),
        ("async def", "    await prepare()\n"),
        ("async def", "    yield 0\n"),
    ] {
        for handler in ["Error", "Error as error"] {
            let source = format!(
                "{definition} f(items):\n{suspension}    try:\n        try:\n            for item in items:\n                try:\n                    work(item)\n                except {handler}:\n                    failed()\n                    return\n        finally:\n            cleanup()\n    finally:\n        finish()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "nested_finally_iterator_return_owners.py",
            );
        }
    }
}

#[test]
fn iterator_returns_in_suspending_exceptional_finally_unwind_before_exception_state() {
    for (name, definition, suspension, loop_kind, returns_value) in [
        ("ordinary", "def", "", "for", true),
        ("generator", "def", "    yield 0\n", "for", true),
        ("coroutine_sync", "async def", "", "for", true),
        ("coroutine_async", "async def", "", "async for", true),
        (
            "async_generator_sync",
            "async def",
            "    yield 0\n",
            "for",
            false,
        ),
        (
            "async_generator_async",
            "async def",
            "    yield 0\n",
            "async for",
            false,
        ),
    ] {
        for (return_name, returned) in [("bare", "return"), ("value", "return item")] {
            if return_name == "value" && !returns_value {
                continue;
            }
            let source = format!(
                "{definition} f(items, work):\n{suspension}    try:\n        work()\n    finally:\n        {loop_kind} item in items:\n            {returned}\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("exceptional_finally_iterator_{name}_{return_name}.py"),
            );
        }
    }
}

#[test]
fn exceptional_finally_iterator_return_preserves_older_and_interleaved_loop_order() {
    for (outer, inner) in [("for", "async for"), ("async for", "for")] {
        let source = format!(
            "async def f(outer, inner, work):\n    {outer} before in outer:\n        try:\n            work()\n        finally:\n            {inner} after in inner:\n                while after:\n                    return after\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("exceptional_finally_older_{outer}_{inner}.py"),
        );
    }
}

#[test]
fn completed_inner_managers_preserve_the_finalizer_line_on_iterator_returns() {
    let source = r"def select(items, resource):
    try:
        for item in items:
            try:
                with resource:
                    return item
            finally:
                release()
    except OSError:
        recover()
";
    common::assert_matches_cpython_marshal(source, "managed_finalizer_return.py");
}
