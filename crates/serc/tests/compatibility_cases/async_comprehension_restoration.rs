use crate::common;

#[test]
fn matches_cpython_async_comprehension_saved_local_restoration() {
    for (opening, closing, element) in [
        ("[", "]", "combine(first, second, third)"),
        ("{", "}", "combine(first, second, third)"),
        ("{", "}", "first: combine(first, second, third)"),
    ] {
        for clauses in [
            "async for first in async_values for second in values for third in values",
            "async for first in async_values for second in [choose(first)] for third in values",
            "async for first, extra in async_values for second in values for third in values",
        ] {
            for target in ["answer", "answer: object"] {
                let source = format!(
                    "async def f(values, async_values):\n    {target} = {opening}{element} {clauses}{closing}\n    return answer\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "async_comprehension_saved_locals.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_nested_async_comprehension_locations() {
    for (opening, closing, element) in [
        ("[", "]", "combine(first, second, third)"),
        ("{", "}", "combine(first, second, third)"),
        ("{", "}", "first: combine(first, second, third)"),
    ] {
        for clauses in [
            "for first in values async for second in async_values for third in values",
            "for first in values for second in values async for third in async_values",
            "for first in values for second in [choose(first)] async for third in async_values",
        ] {
            for assignment in [false, true] {
                let expression = format!("{opening}{element} {clauses}{closing}");
                let body = if assignment {
                    format!("answer = {expression}\n    return answer")
                } else {
                    format!("return {expression}")
                };
                let source = format!("async def f(values, async_values):\n    {body}\n");
                common::assert_matches_cpython_marshal(
                    &source,
                    "nested_async_comprehension_locations.py",
                );
            }
        }
    }
}

#[test]
fn async_comprehension_restores_distinct_outer_values() {
    common::assert_executes(
        "import asyncio\nasync def values():\n    yield 10\nasync def f():\n    first, second, third = 11, 22, 33\n    answer = [(first, second, third) async for first in values() for second in [first + 1] for third in [second + 1]]\n    return answer, (first, second, third)\nprint(asyncio.run(f()))\n",
        "([(10, 11, 12)], (11, 22, 33))\n",
    );
}
