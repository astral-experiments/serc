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

#[test]
fn split_loop_prefix_keeps_original_shared_if_jumps_unlocated() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for loop_header in ["for value in values:", "while active:"] {
            for tail in [
                "if later:\n    consume(value)",
                "while later:\n    consume(value)",
                "assert later",
            ] {
                let tail = tail.replace('\n', "\n        ");
                let source = format!(
                    "{function} f(values, value, active, first, second, third, later):\n{prefix}    {loop_header}\n        if first:\n            if second:\n                work(value)\n            else:\n                if third:\n                    work(value)\n        else:\n            other(value)\n        {tail}\n    after()\n"
                );
                common::assert_matches_cpython_marshal(&source, "split_loop_prefix.py");
            }
        }
    }
}

#[test]
fn split_loop_prefix_preserves_copied_and_unique_exit_locations() {
    for loop_header in ["for value in values:", "while active:"] {
        for inner in [
            "if second:\n    work(value)\nelse:\n    other(value)",
            "if second:\n    work(value)\nelse:\n    if third:\n        return value",
            "if second:\n    work(value)\nelse:\n    if third:\n        work(value)\n    else:\n        return value",
        ] {
            for tail in ["", "\n        if later:\n            return value"] {
                let inner = inner.replace('\n', "\n            ");
                let source = format!(
                    "def f(values, value, active, first, second, third, later):\n    {loop_header}\n        if first:\n            {inner}\n        else:\n            other(value){tail}\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "split_loop_prefix_exit.py");
            }
        }
    }
}

#[test]
fn terminating_arms_preserve_unlocated_nested_else_continuations() {
    for (function, prefix) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
        for head in [
            "if second:\n    return value",
            "if second:\n    raise Error(value)",
            "if second:\n    work(value)\nelif other:\n    return value",
        ] {
            for condition in ["third", "third and other", "third < other < value"] {
                let inner = format!("{head}\nelse:\n    if {condition}:\n        value = third")
                    .replace('\n', "\n        ");
                let source = format!(
                    "{function} f(first, second, third, other, value):\n{prefix}    if first:\n        {inner}\n    else:\n        value = other\n    consume(value)\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "terminating_nested_else.py");
            }
        }
    }
}
