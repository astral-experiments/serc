use crate::common;

#[test]
fn matches_cpython_terminal_named_with_cleanup_copies() {
    for (scope, prefix, with_keyword) in [
        ("def", "", "with"),
        ("async def", "", "with"),
        ("async def", "", "async with"),
        ("def", "    yield seed\n", "with"),
    ] {
        for handler in ["except Error as error", "except Error", "except"] {
            for body in ["recover()", "recovered = recover()"] {
                for successor in ["", "    done()\n"] {
                    let source = format!(
                        "{scope} f(manager, seed):\n{prefix}    try:\n        work()\n    {handler}:\n        {with_keyword} manager:\n            {body}\n{successor}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "terminal_named_with_cleanup.py",
                    );
                }
            }
        }
    }
}

fn indent(source: &str) -> String {
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

#[test]
fn matches_cpython_named_handler_cleanup_after_suppressed_with() {
    for outer in [false, true] {
        for work in ["work()", "with manager:\n    work()"] {
            for preceding_handler in ["", "except Other:\n    report()\n"] {
                for prefix in ["", "            report()\n"] {
                    for returned in ["None", "flag"] {
                        let handler = format!(
                            "try:\n{}{preceding_handler}except Error as error:\n    if flag:\n        with manager:\n{prefix}            raise error\n    else:\n        raise error\n",
                            indent(work)
                        );
                        let body = if outer {
                            format!("if enabled:\n{}", indent(&handler))
                        } else {
                            handler
                        };
                        let source = format!(
                            "def f(flag, enabled, manager):\n{}    return {returned}\n",
                            indent(&body)
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "named_suppressed_with_cleanup.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_cleanup_and_return_owners() {
    for early_return in ["", "    if enabled:\n        return False\n"] {
        for prefix in ["", "        value = compute()\n"] {
            for alias in ["", " as response"] {
                for chain in [false, true] {
                    let returned = if chain {
                        if alias.is_empty() {
                            "0 <= value < 3"
                        } else {
                            "0 <= response.status < 3"
                        }
                    } else {
                        "True"
                    };
                    let source = format!(
                        "def f(manager, value, enabled):\n{early_return}    try:\n        work()\n    except Error:\n{prefix}        try:\n            with manager{alias}:\n                return {returned}\n        except Other:\n            return False\n    return True\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_handler_cleanup.py");
                }
            }
        }
    }
}

#[test]
fn preserves_implicit_returns_after_suppressed_with_handlers() {
    for outer in [false, true] {
        for body in [
            "with manager:\n            raise error",
            "with manager: raise error",
        ] {
            let handler = format!(
                "try:\n    work()\nexcept Error as error:\n    if flag:\n        {body}\n    else:\n        raise error\n"
            );
            let suite = if outer {
                format!("if enabled:\n{}", indent(&handler))
            } else {
                handler
            };
            let source = format!("def f(flag, enabled, manager):\n{}", indent(&suite));
            common::assert_matches_cpython_marshal(&source, "implicit_with_handler_return.py");
        }
    }
}

#[test]
fn matches_cpython_nested_handler_return_owners_after_suppressed_with() {
    for scope in ["def", "async def"] {
        for returned in ["False", "value", "build()"] {
            for prefix in ["", "        value = compute()\n"] {
                for alias in ["", " as response"] {
                    let source = format!(
                        "{scope} f(manager, value):\n    try:\n        work()\n    except Error:\n{prefix}        try:\n            with manager{alias}:\n                return True\n        except Other:\n            return {returned}\n    return True\n",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_handler_suppressed_with_owner.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_pop_preserves_intervening_manager() {
    for returned in ["False", "value"] {
        let source = format!(
            "def process(manager, surrounding, value):\n    try:\n        work()\n    except Error:\n        with surrounding:\n            try:\n                with manager:\n                    return True\n            except Other:\n                return {returned}\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_handler_intervening_manager.py");
    }
}
