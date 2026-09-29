use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_call_name_condition_owners() {
    for prefix in ["", "    await initialize()\n"] {
        for condition in ["should_continue()", "should_continue(item)", "check(item)"] {
            for body in [
                "        item = await consume(item)\n",
                "        assert 0 < item < 8\n",
                "        yield item\n        item = await consume(item)\n",
            ] {
                let source = format!(
                    "async def f(item, check, consume):\n{prefix}    while {condition}:\n{body}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "coroutine_call_name_condition_owners.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_generator_method_condition_owners() {
    for (declaration, suffix) in [
        ("def", "    yield first\n"),
        ("async def", ""),
        ("async def", "    yield first\n"),
    ] {
        for condition in [
            "reader.ready(first, second)",
            "get_reader().ready(first)",
            "reader.ready((1, 2))",
            "reader.ready(first, option=second)",
        ] {
            let source = format!(
                "{declaration} f(reader, first, second):\n    while {condition}:\n        consume(first)\n{suffix}"
            );
            common::assert_matches_cpython_marshal(&source, "generator_method_condition_owners.py");
        }
    }
}
