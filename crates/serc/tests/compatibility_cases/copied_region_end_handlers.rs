use crate::common;

#[test]
fn matches_cpython_async_with_return_handler_after_iteration() {
    for (filename, body, handler) in [
        ("generator_return_handler.py", "yield item", "return"),
        ("generator_raise_handler.py", "yield item", "raise"),
        ("coroutine_return_handler.py", "consume(item)", "return"),
        ("coroutine_raise_handler.py", "consume(item)", "raise"),
    ] {
        let source = format!(
            "async def f(manager, values):\n    try:\n        async with manager:\n            async for item in values:\n                {body}\n    except Error:\n        {handler}\n",
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}
