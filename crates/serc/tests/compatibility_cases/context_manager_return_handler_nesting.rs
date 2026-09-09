use crate::common;

#[test]
fn matches_cpython_return_handler_cleanup_inside_context_managers() {
    for (function, manager, work) in [
        ("def", "with", "work()"),
        ("async def", "with", "await work()"),
        ("async def", "async with", "await work()"),
    ] {
        for named in [false, true] {
            for preceding in ["", "            record()\n"] {
                for value in ["None", "value", "value.result", "build(value)"] {
                    let binding = if named { " as error" } else { "" };
                    let value = if named && value == "value.result" {
                        "error.result"
                    } else {
                        value
                    };
                    let source = format!(
                        "{function} f(manager, value):\n    {manager} manager:\n        try:\n            return {work}\n        except Error{binding}:\n{preceding}            return {value}\n        except Other:\n            raise\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "context_manager_return_handler_nesting.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_return_handler_cleanup_after_inner_context_managers() {
    for value in ["None", "value", "value.result", "build(value)"] {
        let source = format!(
            "def f(manager, value):\n    try:\n        work()\n    except Error:\n        with manager:\n            return {value}\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "return_handler_after_inner_context_manager.py",
        );
    }
}

#[test]
fn matches_cpython_return_handler_cleanup_with_enclosing_exception_regions() {
    for named in [false, true] {
        for value in ["None", "value", "value.result", "build(value)"] {
            let binding = if named { " as error" } else { "" };
            let body = format!(
                "            try:\n                return work()\n            except Error{binding}:\n                record()\n                return {value}\n            except Other:\n                raise\n"
            );
            for source in [
                format!(
                    "def f(manager, value):\n    try:\n        with manager:\n{body}    except Root:\n        raise\n"
                ),
                format!(
                    "def f(manager, value):\n    with manager:\n        try:\n{body}        except Root:\n            raise\n"
                ),
                format!(
                    "def f(manager, value):\n    try:\n        work()\n    except Root:\n        with manager:\n{body}"
                ),
            ] {
                common::assert_matches_cpython_marshal(
                    &source,
                    "return_handler_enclosing_exception_regions.py",
                );
            }
        }
    }
}
