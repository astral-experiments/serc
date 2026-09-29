use crate::common;

#[test]
fn terminal_named_handler_false_exits_keep_the_sole_operand_location() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for condition in [
            "a or b",
            "not (a and b)",
            "(\n            a\n            or b\n        )",
        ] {
            for tail in ["", "        elif b or c:\n            recover(error)\n"] {
                let source = format!(
                    "{function} f(a, b, c):\n{prefix}    try:\n        observe()\n    except Exception as error:\n        if {condition}:\n            report(error)\n{tail}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminal_named_handler_false_locations.py",
                );
            }
        }
    }
}

#[test]
fn prefixed_named_handler_boolean_exits_keep_predecessor_order() {
    for scope in [
        "",
        "def f(value, flag, other, ready):\n",
        "async def f(value, flag, other, ready):\n",
    ] {
        for prefix in ["report(value)", "value = transform(value)"] {
            for condition in [
                "flag and other",
                "flag and other and ready",
                "(flag or other) and ready",
            ] {
                for body in ["report(value)", "import math\n        report(math)", "pass"] {
                    let handler = format!(
                        "try:\n    observe(value)\nexcept Exception as error:\n    {prefix}\n    if {condition}:\n        {body}\n",
                    );
                    let source = if scope.is_empty() {
                        handler
                    } else {
                        format!("{scope}    {}", handler.trim_end().replace('\n', "\n    "))
                    };
                    common::assert_matches_cpython_marshal(
                        &source,
                        "named_handler_boolean_cleanup_order.py",
                    );
                }
            }
        }
    }
}
