#[allow(dead_code)]
mod common;

#[test]
fn specialized_none_guard_keeps_private_handler_backedges() {
    for condition in [
        "value is not None and check(value)",
        "value is None and check(value)",
        "check(value) and value is not None",
        "check(value)",
    ] {
        let source = format!(
            "def f(value):\n    while True:\n        try:\n            work()\n        except Error:\n            if {condition}:\n                break\n        except Exception:\n            break\n"
        );
        common::assert_matches_cpython_marshal(&source, "handler_backedges.py");
    }
}

#[test]
fn nested_none_guard_preserves_named_cleanup_copy_order() {
    for suspension in ["", "await "] {
        for test in ["replacement is not None", "replacement is None"] {
            let source = format!(
                "async def f(enabled, value, replacement):\n    try:\n        work()\n    except Error as error:\n        if enabled:\n            if value is None:\n                if {test}:\n                    try:\n                        {suspension}work()\n                    except Exception:\n                        recover()\n            else:\n                finish()\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_cleanup_copy.py");
        }
    }
}

#[test]
fn protected_while_break_keeps_its_nop_before_empty_normal_exit() {
    let source = r"def f(first, second, value):
    try:
        while first and second:
            if value is None:
                break
    except RuntimeError as error:
        handle(error)
    if first or second:
        use(value)
";
    common::assert_matches_cpython_marshal(source, "protected_while_break_nop.py");
}
