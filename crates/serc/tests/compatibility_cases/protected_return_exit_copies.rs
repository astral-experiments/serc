use crate::common;

#[test]
fn matches_cpython_protected_return_exit_copies() {
    for prefix in ["", "async "] {
        for branches in [
            "if flag:\n    consume()\nelse:\n    consume(other)",
            "if flag:\n    consume()\nelif other:\n    consume(other)\nelse:\n    consume(value)",
        ] {
            for value in ["value", "None", "[value]", "make_value(value)"] {
                let source = format!(
                    "{prefix}def f(value, flag, other):\n    try:\n        {}\n        return {value}\n    except Error:\n        raise\n",
                    branches.replace('\n', "\n        "),
                );
                common::assert_matches_cpython_marshal(&source, "protected_return_exit_copies.py");
            }
        }
    }
}

#[test]
fn matches_cpython_conditional_nested_handler_return_copies() {
    for prefix in ["", "async "] {
        for outer_return in ["", "return "] {
            for value in ["value", "make_value(value)"] {
                for conditional in [false, true] {
                    let body = format!("try:\n    return {value}\nexcept Error:\n    pass");
                    let body = if conditional {
                        format!("if condition:\n    {}", body.replace('\n', "\n    "))
                    } else {
                        body
                    };
                    let source = format!(
                        "{prefix}def f(value, condition):\n    try:\n        {outer_return}work(value)\n    except Error:\n        {}\n    return value\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "conditional_nested_handler_return_copies.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_manager_exits_before_protected_terminal_breaks() {
    for prefix in ["", "async "] {
        for body in ["consume(value)", "yield value"] {
            for manager in [
                format!("{prefix}with manager():\n    {body}"),
                format!("{prefix}with manager():\n    {prefix}with another():\n        {body}"),
            ] {
                let source = format!(
                    "{prefix}def f(value):\n    while True:\n        try:\n            {}\n            break\n        except Error:\n            continue\n",
                    manager.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "manager_break_exit_copies.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_try_iterator_conditional_return_target() {
    let source = r"def fetch_parsed(attempts, fetch, decode, pause):
    last_error = None
    for attempt in attempts:
        try:
            payload = fetch()
            try:
                return decode(payload) if payload else {}
            except ValueError as error:
                raise RuntimeError('decode failed') from error
        except Exception as error:
            last_error = error
            pause(attempt)
    raise RuntimeError('attempts exhausted') from last_error
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_try_iterator_conditional_return_target.py",
    );
}

#[test]
fn matches_cpython_single_try_iterator_conditional_return_control() {
    let source = r"def fetch_parsed(attempts, fetch, decode, pause):
    last_error = None
    for attempt in attempts:
        try:
            payload = fetch()
            return decode(payload) if payload else {}
        except Exception as error:
            last_error = error
            pause(attempt)
    raise RuntimeError('attempts exhausted') from last_error
";
    common::assert_matches_cpython_marshal(
        source,
        "single_try_iterator_conditional_return_control.py",
    );
}
