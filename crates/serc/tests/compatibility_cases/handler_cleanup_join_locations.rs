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

#[test]
fn matches_cpython_handler_cleanup_after_for_else_raise() {
    for binding in ["", " as error"] {
        for raising in ["raise", "raise Error(b)", "raise Error(b) from cause"] {
            for body in [
                "if a:\n    value = item\n    break",
                "if a:\n    value = item\n    break\nif b:\n    value = None\n    break",
            ] {
                let source = format!(
                    "def f(items, a, b):\n    try:\n        value = work()\n    except Error{binding}:\n        for item in items:\n            {}\n        else:\n            {raising}\n    return value\n",
                    body.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "handler_for_else_raise.py");
            }
        }
    }
}

#[test]
fn matches_cpython_for_else_handler_cleanup_in_suspended_and_finally_contexts() {
    for source in [
        r"def f(items, a, b):
    try:
        value = work()
    except Error:
        for item in items:
            try:
                value = item
                break
            finally:
                cleanup()
        else:
            raise
    return value
",
        r"def f(items, a, b):
    yield None
    try:
        value = work()
    except Error:
        for item in items:
            if a:
                value = item
                break
            if b:
                value = None
                break
        else:
            raise
    return value
",
        r"async def f(items, a, b):
    try:
        value = work()
    except Error:
        async for item in items:
            value = item
            break
        else:
            raise
    return value
",
    ] {
        for binding in ["", " as error"] {
            let source = source.replace("except Error:", &format!("except Error{binding}:"));
            common::assert_matches_cpython_marshal(&source, "handler_for_else_contexts.py");
        }
    }
}

#[test]
fn matches_cpython_handler_cleanup_after_break_in_finally_body() {
    for binding in ["", " as error"] {
        for raising in [
            "raise",
            "raise Error(wanted)",
            "report(value)\nraise Error(wanted)",
        ] {
            for enclosing_handler in [false, true] {
                let body = format!(
                    "try:\n    value = work()\nexcept Error{binding}:\n    for item in items:\n        try:\n            value = transform(item)\n        finally:\n            if condition:\n                break\n    else:\n        {}\nreturn value",
                    raising.replace('\n', "\n        "),
                );
                let body = if enclosing_handler {
                    format!(
                        "try:\n    {}\nexcept Other as outer_error:\n    report(outer_error)\n    raise",
                        body.replace('\n', "\n    "),
                    )
                } else {
                    body
                };
                let source = format!(
                    "def f(items, wanted, condition):\n    {}\n",
                    body.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "handler_cleanup_after_break_in_finally.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_transitively_copied_handler_exclusions() {
    for named in [false, true] {
        for tail in [
            "consume(value)",
            "if not valid(value):\n    raise ValueError()",
        ] {
            let mut body = "value = last_attempt()".to_string();
            for index in (0..4).rev() {
                let binding = if named {
                    format!(" as error_{index}")
                } else {
                    String::new()
                };
                body = format!(
                    "try:\n    value = attempt_{index}()\nexcept Error{binding}:\n    {}",
                    body.replace('\n', "\n    "),
                );
            }
            let body = format!("{body}\n{tail}\nreturn value");
            for context in ["plain", "generator", "manager"] {
                let body = match context {
                    "generator" => format!("yield 0\n{body}"),
                    "manager" => format!("with manager():\n    {}", body.replace('\n', "\n    ")),
                    _ => body.clone(),
                };
                let source = format!("def f():\n    {}\n", body.replace('\n', "\n    "));
                common::assert_matches_cpython_marshal(
                    &source,
                    "transitively_copied_handler_exclusions.py",
                );
            }
        }
    }
}
