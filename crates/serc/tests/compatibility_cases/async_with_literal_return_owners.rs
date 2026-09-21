use crate::common;
use crate::common::indent;

fn protected(body: &str) -> String {
    format!("try:\n{}except Error:\n    fallback()\n", indent(body))
}

#[test]
fn matches_cpython_async_with_literal_return_owners() {
    for returned in [
        "return None",
        "return 7",
        "return True",
        "return ()",
        "return",
    ] {
        for prefix in ["", "await work()\n"] {
            for conditional in [false, true] {
                for nested in [false, true] {
                    let body = if conditional {
                        format!("{prefix}if enabled:\n    {returned}\nother()\n")
                    } else {
                        format!("{prefix}{returned}\n")
                    };
                    let mut body = protected(&format!("async with manager:\n{}", indent(&body)));
                    if nested {
                        body = format!("try:\n{}except Other:\n    fallback()\n", indent(&body));
                    }
                    let source =
                        format!("async def f(manager, value, enabled):\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(&source, "async_with_literal_return.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_async_generator_with_bare_return_owners() {
    for prefix in ["", "yield value\n"] {
        for conditional in [false, true] {
            for nested in [false, true] {
                let body = if conditional {
                    format!("{prefix}if enabled:\n    return\nother()\n")
                } else {
                    format!("{prefix}return\n")
                };
                let mut body = protected(&format!("async with manager:\n{}", indent(&body)));
                if nested {
                    body = format!("try:\n{}except Other:\n    fallback()\n", indent(&body));
                }
                let source = format!(
                    "async def f(manager, value, enabled):\n    yield value\n{}",
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "async_generator_with_bare_return.py",
                );
            }
        }
    }
}

#[test]
fn preserves_async_with_value_and_finally_return_owners() {
    for returned in ["return value", "return call()"] {
        for conditional in [false, true] {
            for finalizer in [false, true] {
                let body = if conditional {
                    format!("if enabled:\n    {returned}\nother()\n")
                } else {
                    format!("{returned}\n")
                };
                let body = format!("async with manager:\n{}", indent(&body));
                let body = if finalizer {
                    format!("try:\n{}finally:\n    cleanup()\n", indent(&body))
                } else {
                    protected(&body)
                };
                let source = format!("async def f(manager, value, enabled):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(&source, "async_with_value_return.py");
            }
        }
    }
}

#[test]
fn preserves_async_with_iterator_literal_return_owners() {
    for returned in ["return None", "return 7", "return", "return value"] {
        for conditional in [false, true] {
            for manager_inside_iterator in [false, true] {
                let body = if conditional {
                    format!("if enabled:\n    {returned}\nother()\n")
                } else {
                    format!("{returned}\n")
                };
                let body = protected(&body);
                let body = if manager_inside_iterator {
                    let body = format!("async with manager:\n{}", indent(&body));
                    format!("for item in choices:\n{}", indent(&body))
                } else {
                    let body = format!("for item in choices:\n{}", indent(&body));
                    format!("async with manager:\n{}", indent(&body))
                };
                let source = format!(
                    "async def f(manager, other_manager, choices, value, enabled):\n{}",
                    indent(&protected(&body)),
                );
                common::assert_matches_cpython_marshal(&source, "async_with_iterator_return.py");
            }
        }
    }
}
