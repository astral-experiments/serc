use crate::common;

#[test]
fn matches_cpython_protected_while_call_folds() {
    for (prefix, initial, manager) in [
        ("", "", "with"),
        ("", "    yield\n", "with"),
        ("async ", "", "with"),
        ("async ", "", "async with"),
        ("async ", "    yield\n", "async with"),
    ] {
        for argument in ["8 * 1024 * 1024", "8388608", "2 * 3"] {
            for assignment in ["", "chunk := "] {
                for nested in [false, true] {
                    let body = format!("while {assignment}source.read({argument}):\n    consume()");
                    let body = if nested {
                        format!("with another:\n    {}", body.replace('\n', "\n    "))
                    } else {
                        body
                    };
                    let source = format!(
                        "{prefix}def f(source, manager, another):\n{initial}    {manager} manager:\n        {}\n",
                        body.replace('\n', "\n        "),
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "protected_while_call_folds.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_protected_while_call_folds_before_terminal_bodies() {
    for (prefix, initial, manager) in [
        ("", "", "with"),
        ("", "    yield\n", "with"),
        ("async ", "", "with"),
        ("async ", "", "async with"),
    ] {
        for argument in ["10", "2 * 3"] {
            for assignment in ["", "chunk := "] {
                for body in ["consume()\n            break", "return", "raise Error"] {
                    let source = format!(
                        "{prefix}def f(source, manager):\n{initial}    {manager} manager:\n        while {assignment}source.read({argument}):\n            {body}\n",
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "protected_while_terminal_call_folds.py",
                    );
                }
            }
        }
    }
}
