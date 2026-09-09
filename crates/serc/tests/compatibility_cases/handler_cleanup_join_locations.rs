use crate::common;

#[test]
fn matches_cpython_nested_handler_fallthrough_cleanup_copies() {
    for (definition, prologue) in [("def", ""), ("async def", ""), ("def", "    yield None\n")] {
        for binding in ["", " as error"] {
            for body in ["result = work(value)", "work(value)"] {
                for handler in ["pass", "recover(value)"] {
                    for in_loop in [false, true] {
                        let recovery = format!(
                            "try:\n    prepare(value)\nexcept Error:\n    try:\n        {body}\n    except Other{binding}:\n        {handler}"
                        );
                        let recovery = if in_loop {
                            format!(
                                "for value in items:\n    {}",
                                recovery.replace('\n', "\n    "),
                            )
                        } else {
                            recovery
                        };
                        let source = format!(
                            "{definition} f(items, value):\n{prologue}    {}\n    finish()\n",
                            recovery.replace('\n', "\n    "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "nested_handler_fallthrough_cleanup_copies.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_handler_cleanup_join_locations() {
    let source = r"def conditional_handler_before_finally(condition):
    try:
        work()
    except Error as error:
        report(error)
        if condition:
            action()
    finally:
        cleanup()

async def conditional_async_handler_before_finally(condition):
    try:
        await work()
    except Error as error:
        report(error)
        if condition:
            await action()
    finally:
        cleanup()

def conditional_with_in_handler(condition, manager):
    try:
        work()
    except Error as error:
        with manager:
            if condition:
                action()
    finish()
";
    common::assert_matches_cpython_marshal(source, "handler_cleanup_join_locations.py");
}

#[test]
fn preserves_conditional_handler_cleanup_locations_at_loop_backedges() {
    for (definition, prologue) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield None\n"),
        ("async def", "    yield None\n"),
    ] {
        for loop_header in ["for value in values", "while flag"] {
            for name in ["", " as error"] {
                for following in ["", "    finish()\n"] {
                    let source = format!(
                        "{definition} process(values, value, flag):\n{prologue}    {loop_header}:\n        try:\n            work()\n        except Error{name}:\n            if check(value):\n                recover(value)\n{following}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "conditional_handler_loop_cleanup.py",
                    );
                }
            }
        }
    }
}
