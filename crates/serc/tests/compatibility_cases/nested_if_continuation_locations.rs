use crate::common;

#[test]
fn inherited_if_continuations_keep_shared_jumps_unlocated() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for depth in [1, 3] {
            for condition in ["third", "third and fourth", "third < fourth < value"] {
                let mut body = format!(
                    "if first:\n    if second:\n        work(value)\n    else:\n        if {condition}:\n            work(value)\nelse:\n    other()\n"
                );
                for _ in 0..depth {
                    body = format!(
                        "if outer:\n    {}\n",
                        body.trim_end().replace('\n', "\n    ")
                    );
                }
                body.push_str("after()\n");
                let source = format!(
                    "{function} f(outer, first, second, third, fourth, value):\n{prefix}    {}\n",
                    body.trim_end().replace('\n', "\n    ")
                );
                common::assert_matches_cpython_marshal(&source, "inherited_if_continuation.py");
            }
        }
    }
}

#[test]
fn terminal_nested_if_keeps_return_copy_locations() {
    for function in ["def", "async def"] {
        for depth in [1, 3] {
            let mut body = "if first:\n    if second:\n        work()\n    else:\n        if third:\n            work()\nelse:\n    other()\n".to_owned();
            for _ in 0..depth {
                body = format!(
                    "if outer:\n    {}\n",
                    body.trim_end().replace('\n', "\n    ")
                );
            }
            let source = format!(
                "{function} f(outer, first, second, third):\n    {}\n",
                body.trim_end().replace('\n', "\n    ")
            );
            common::assert_matches_cpython_marshal(&source, "terminal_nested_if.py");
        }
    }
}

#[test]
fn terminal_nested_arms_preserve_shared_exit_condition_locations() {
    for (function, prefix) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
        for condition in [
            "second",
            "not second",
            "second and fourth",
            "second or fourth",
        ] {
            for body in [
                "if third:\n    work(value)\nelse:\n    raise Error(value)",
                "if third:\n    work(value)\nelse:\n    if fourth:\n        return value\n    else:\n        raise Error(value)",
                "assert False",
            ] {
                let body = body.replace('\n', "\n            ");
                let source = format!(
                    "{function} f(first, second, third, fourth, value):\n{prefix}    if first:\n        work(value)\n        if {condition}:\n            {body}\n    else:\n        other(value)\n    consume(value)\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "terminal_nested_arms.py");
            }
        }
    }
}
