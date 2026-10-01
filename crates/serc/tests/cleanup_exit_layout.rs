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
fn method_guarded_finalizer_copies_each_iterator_discard() {
    for guard in ["value.check()", "item.check()"] {
        let source = format!(
            "def f(items, value):\n    for item in items:\n        try:\n            work()\n        except Error as error:\n            if item:\n                break\n            recover()\n        finally:\n            if {guard}:\n                value.finish()\n    assert value is not None\n    raise value\n"
        );
        common::assert_matches_cpython_marshal(&source, "guarded_finally_iterator.py");
    }
}

#[test]
fn normal_finalizer_donates_the_manager_exit_marker() {
    for cleanup in ["value.attr = None", "value.finish()", "result = 0"] {
        let source = format!(
            "def f(value, manager):\n    while value:\n        with manager:\n            if value:\n                try:\n                    work()\n                finally:\n                    {cleanup}\n            else:\n                wait()\n                continue\n        finish()\n"
        );
        common::assert_matches_cpython_marshal(&source, "finally_manager_exit.py");
    }
}
