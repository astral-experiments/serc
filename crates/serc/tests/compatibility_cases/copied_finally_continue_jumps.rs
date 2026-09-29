use crate::common;

#[test]
fn matches_cpython_copied_finally_continue_handler_jumps() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for cleanup in [
                "try:\n    cleanup(value)\nexcept Error:\n    pass",
                "try:\n    cleanup(value)\nexcept Error as error:\n    pass",
                "try:\n    cleanup(value)\nexcept:\n    pass",
                "try:\n    cleanup(value)\nexcept Error:\n    failed(value)",
                "try:\n    cleanup(value)\nexcept Error:\n    pass\nelse:\n    success(value)",
            ] {
                let cleanup = cleanup.replace('\n', "\n            ");
                let source = format!(
                    "{definition} f(values, flag, manager):\n    {loop_header}:\n        try:\n            consume(value)\n            continue\n        finally:\n            {cleanup}\n{footer}    return values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_finally_continue_handler.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_copied_finally_continue_nested_cleanup() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for cleanup in [
                "try:\n    cleanup(value)\nfinally:\n    try:\n        other(value)\n    except Error:\n        pass",
                "try:\n    cleanup(value)\nexcept Error:\n    pass\nfinally:\n    other(value)",
                "with manager:\n    try:\n        cleanup(value)\n    except Error:\n        pass",
                "for nested in values:\n    try:\n        cleanup(nested)\n    except Error:\n        pass",
            ] {
                let cleanup = cleanup.replace('\n', "\n            ");
                let source = format!(
                    "{definition} f(values, flag, manager):\n    {loop_header}:\n        try:\n            consume(value)\n            continue\n        finally:\n            {cleanup}\n{footer}    return values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_finally_continue_nested.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_continue_stays_inside_its_finally_context() {
    for (definition, footer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield values\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for body in [
                "try:\n    for nested in values:\n        consume(value)\n        continue\nfinally:\n    try:\n        cleanup(value)\n    except Error:\n        pass",
                "try:\n    consume(value)\nfinally:\n    for nested in values:\n        consume(value)\n        continue",
            ] {
                let body = body.replace('\n', "\n        ");
                let source = format!(
                    "{definition} f(values, flag, manager):\n    {loop_header}:\n        {body}\n{footer}    return values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "continue_within_finally_context.py",
                );
            }
        }
    }
}
