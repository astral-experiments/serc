use std::fmt::Write as _;

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

#[test]
fn owner_transfer_nested_manager_depth_matrix() {
    for outer_depth in [1, 2] {
        for inner_depth in [0, 1, 2] {
            for handler in ["except:", "except Error:"] {
                for value in ["value", "None"] {
                    let mut source = String::from("def f(a, b, c, d, value, ignore):\n");
                    let mut indent = String::from("    ");
                    for manager in ["a", "b"].into_iter().take(outer_depth) {
                        writeln!(source, "{indent}with {manager}:").unwrap();
                        indent.push_str("    ");
                    }
                    writeln!(source, "{indent}try:").unwrap();
                    let mut body_indent = format!("{indent}    ");
                    for manager in ["c", "d"].into_iter().take(inner_depth) {
                        writeln!(source, "{body_indent}with {manager}:").unwrap();
                        body_indent.push_str("    ");
                    }
                    writeln!(source, "{body_indent}return {value}").unwrap();
                    writeln!(source, "{indent}{handler}\n{indent}    if ignore:\n{indent}        return None\n{indent}    else:\n{indent}        raise").unwrap();
                    common::assert_matches_cpython_marshal(
                        &source,
                        "owner_transfer_nested_manager_depth_matrix.py",
                    );
                }
            }
        }
    }
}

#[test]
fn owner_transfer_async_manager_depth_matrix() {
    for outer in ["with", "async with"] {
        for inner in ["with", "async with"] {
            let source = format!(
                "async def f(a, b, value, ignore):\n    {outer} a:\n        try:\n            {inner} b:\n                return value\n        except:\n            if ignore:\n                return None\n            else:\n                raise\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "owner_transfer_async_manager_depth_matrix.py",
            );
        }
    }
}
