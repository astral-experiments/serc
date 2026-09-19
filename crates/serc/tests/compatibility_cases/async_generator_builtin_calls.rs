use crate::common;

#[test]
fn matches_cpython_builtin_calls_with_async_generator_expressions() {
    for builtin in ["any", "all", "tuple"] {
        for generator in [
            "item for item in stream",
            "item async for item in stream",
            "await transform(item) for item in stream",
            "item for item in stream if await predicate(item)",
            "item for group in stream for item in await acquire(group)",
            "item for group in stream async for item in group",
            "item for item in await acquire()",
            "[x async for x in item] for item in stream",
            "(x async for x in item) for item in stream",
            "(x for x in await acquire(item)) for item in stream",
            "(lambda x=await transform(item): x) for item in stream",
            "(x for x in [z async for z in item]) for item in stream",
            "(lambda x=[z async for z in item]: x) for item in stream",
            "True for (await get_target()).value in stream",
            "True for obj[await get_index()] in stream",
        ] {
            for preceding_await in ["", "    await tick()\n"] {
                let source = format!(
                    "async def function(stream):\n{preceding_await}    result = {builtin}({generator})\n    consume(result)\n"
                );
                common::assert_matches_cpython_marshal(&source, "async_generator_builtin_calls.py");
            }
        }
    }
}
