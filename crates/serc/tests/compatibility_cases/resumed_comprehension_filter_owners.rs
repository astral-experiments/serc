use crate::common;

#[test]
fn matches_cpython_comprehension_filters_after_suspension() {
    for (definition, suspension) in [
        ("def", "yield 1"),
        ("async def", "await prepare()"),
        ("async def", "yield 1"),
    ] {
        for condition in ["flag", "item is not None"] {
            let body =
                format!("result = [item for item in items if {condition}]\nconsume(result)\n");
            for (opening, closing) in [
                ("try:", "    except Error:\n        pass\n"),
                ("try:", "    finally:\n        cleanup()\n"),
                ("with manager():", ""),
                (
                    "try:\n        with manager():",
                    "    except Error:\n        pass\n",
                ),
            ] {
                let indent = if opening.contains('\n') { 12 } else { 8 };
                let mut source =
                    format!("{definition} f(items, flag):\n    {suspension}\n    {opening}\n");
                for line in body.lines() {
                    source.push_str(&" ".repeat(indent));
                    source.push_str(line);
                    source.push('\n');
                }
                source.push_str(closing);
                common::assert_matches_cpython_marshal(&source, "resumed_filters.py");
            }
        }
    }
}

#[test]
fn matches_cpython_filters_in_finally_and_handler_bodies() {
    for (definition, suspension) in [
        ("def", "prepare()"),
        ("def", "yield 1"),
        ("async def", "await prepare()"),
        ("async def", "yield 1"),
    ] {
        for condition in ["flag", "item is not None"] {
            for (body, handler) in [
                ("prepare()", "finally"),
                ("return", "finally"),
                ("prepare()", "except Error"),
            ] {
                let source = format!(
                    "{definition} f(items, flag):\n    {suspension}\n    try:\n        {body}\n    {handler}:\n        result = [item for item in items if {condition}]\n        consume(result)\n"
                );
                common::assert_matches_cpython_marshal(&source, "cleanup_filters.py");
            }
        }
    }
}

#[test]
fn matches_cpython_filters_in_nested_async_with_regions() {
    for suspension in ["prepare()", "await prepare()", "yield 1"] {
        for condition in ["flag", "item is not None"] {
            for handler in [
                "except Error:\n            pass",
                "finally:\n            cleanup()",
            ] {
                let source = format!(
                    "async def f(items, flag):\n    {suspension}\n    async with manager():\n        try:\n            result = [item for item in items if {condition}]\n            consume(result)\n        {handler}\n"
                );
                common::assert_matches_cpython_marshal(&source, "async_with_filters.py");
            }
        }
    }
}
