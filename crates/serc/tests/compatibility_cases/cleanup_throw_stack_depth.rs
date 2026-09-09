use crate::common;

#[test]
fn matches_cpython_async_generator_folded_loop_cleanup() {
    for condition in ["False", "0", "not True"] {
        for prefix in ["", "consume(item)\n", "await wait(item)\n"] {
            let body =
                format!("{prefix}if {condition}:\n    yield item").replace('\n', "\n        ");
            let source =
                format!("async def f(values):\n    async for item in values:\n        {body}\n");
            common::assert_matches_cpython_marshal(&source, "async_generator_folded_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_live_cleanup_depth_after_dead_calls() {
    for (definition, suspension) in [
        ("async def", "await work(values)\n"),
        ("async def", "async for item in values:\n    pass\n"),
        ("def", "yield from values\n"),
    ] {
        for argument_count in [1, 2, 3, 5] {
            for branch in ["if False", "while False"] {
                let arguments = vec!["flag"; argument_count].join(", ");
                let body = format!("{suspension}{branch}:\n    consume({arguments})\n")
                    .replace('\n', "\n    ");
                let source = format!("{definition} f(values, flag):\n    {body}\n");
                common::assert_matches_cpython_marshal(&source, "live_cleanup_after_dead_call.py");
            }
        }
    }
}

#[test]
fn matches_cpython_folded_loop_cleanup_with_extended_arguments() {
    for (definition, body) in [
        ("def", "yield item"),
        ("async def", "yield item"),
        ("async def", "consume(item)"),
    ] {
        for loop_header in ["for item in values", "while flag"] {
            let prefix = "        consume(item)\n".repeat(60);
            let source = format!(
                "{definition} f(values, flag):\n    {loop_header}:\n{prefix}        if False:\n            {body}\n"
            );
            common::assert_matches_cpython_marshal(&source, "extended_folded_loop_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_unreachable_cleanup_does_not_raise_stack_bound() {
    for source in [
        "async def f(values):\n    if False:\n        await work(values)\n",
        "def f(values):\n    if False:\n        yield from values\n",
    ] {
        common::assert_matches_cpython_marshal(source, "unreachable_suspension_cleanup.py");
    }
}
