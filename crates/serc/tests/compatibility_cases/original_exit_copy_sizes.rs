use crate::common;

#[test]
fn matches_cpython_nested_try_return_copy_sizes() {
    for prefix in ["", "async "] {
        for depth in 1..=3 {
            for call in [
                "factory(value=value if flag else None)",
                "factory(1, value=value if flag else None)",
            ] {
                let mut body = format!("return {call}");
                for _ in 0..depth {
                    body = format!(
                        "try:\n    {}\nexcept Error:\n    recover()",
                        body.replace('\n', "\n    "),
                    );
                }
                let source = format!(
                    "{prefix}def f(flag, value):\n    {}\n",
                    body.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "nested_try_return_copy_sizes.py");
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_try_footer_copy_sizes() {
    for prefix in ["", "async "] {
        for tail in ["del value", "consume(value)"] {
            for handler in ["raise", "recover()"] {
                let source = format!(
                    "{prefix}def f(flag, value):\n    try:\n        if flag:\n            consume(value)\n        else:\n            other(value)\n        {tail}\n    except Error:\n        {handler}\n",
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminal_try_footer_copy_sizes.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_async_generator_nested_try_return_copy_size() {
    common::assert_matches_cpython_marshal(
        r"async def f(values, flag):
    ready = False
    try:
        try:
            async for value in values:
                if flag:
                    continue
                yield value
            if flag:
                yield 1
            try:
                await work()
                ready = True
            except Error as error:
                await recover(error)
                ready = True
                raise
            return
        except OtherError as error:
            await recover(error)
            ready = True
            if flag:
                raise
            consume(error)
        async for value in values:
            yield value
    except Error as error:
        if not ready:
            await recover(error)
        raise
",
        "async_generator_nested_try_return_copy_size.py",
    );
}
