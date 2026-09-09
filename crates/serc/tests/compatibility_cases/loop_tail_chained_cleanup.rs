use crate::common;

#[test]
fn matches_cpython_loop_tail_negated_chained_cleanup() {
    for header in ["for value in items:", "while ready():"] {
        for condition in [
            "not 0 <= value <= 1",
            "value is not None and not 0 <= value <= 1",
            "flag or not 0 <= value <= 1",
        ] {
            for body in ["consume(value)", "raise ValueError(value)", "return None"] {
                let source = format!(
                    "def f(items, value, flag):\n    {header}\n        if {condition}:\n            {body}\n    finish()\n"
                );
                common::assert_matches_cpython_marshal(&source, "loop_tail_chained_cleanup.py");
            }
        }
    }
}

#[test]
fn matches_cpython_loop_tail_boolean_chained_cleanup() {
    for header in ["for value in items:", "while ready():"] {
        for body in ["consume(value)", "raise ValueError(value)", "continue"] {
            let source = format!(
                "def f(items, value):\n    {header}\n        if value is not None and 0 <= value <= 1:\n            {body}\n    finish()\n"
            );
            common::assert_matches_cpython_marshal(&source, "loop_tail_boolean_chained_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_loop_tail_chained_none_cleanup() {
    for header in ["for value in items:", "while ready():"] {
        for condition in ["left is value is None", "not left is value is None"] {
            let source = format!(
                "def f(items, value, left):\n    {header}\n        if {condition}:\n            consume(value)\n    finish()\n"
            );
            common::assert_matches_cpython_marshal(&source, "loop_tail_chained_none_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_while_break_chained_cleanup() {
    for condition in [
        "not 0 <= value <= 1",
        "value is not None and not 0 <= value <= 1",
        "flag or not 0 <= value <= 1",
    ] {
        let source = format!(
            "def f(value, flag):\n    while ready():\n        if {condition}:\n            break\n    finish()\n"
        );
        common::assert_matches_cpython_marshal(&source, "while_break_chained_cleanup.py");
    }
}

#[test]
fn matches_cpython_loop_tail_chained_threaded_owners() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", "        await pause()\n"),
        ("", "        yield\n"),
        ("async ", "        yield\n"),
    ] {
        for header in ["while True:", "for item in items:"] {
            for condition in [
                "0 <= value < limit",
                "not (0 <= value < limit)",
                "left is value is None",
                "not (left is value is None)",
            ] {
                let source = format!(
                    "{prefix}def f(items, left, value, limit):\n    {header}\n{suspension}        tick()\n        if {condition}:\n            report()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "loop_tail_chained_threaded_owners.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_protected_loop_tail_chained_threaded_owners() {
    for (prefix, suspension) in [
        ("", ""),
        ("async ", "            await pause()\n"),
        ("", "            yield\n"),
        ("async ", "            yield\n"),
    ] {
        for (region, cleanup) in [
            ("with manager:", ""),
            ("try:", "    finally:\n        cleanup()\n"),
        ] {
            for condition in ["0 <= value < limit", "not (0 <= value < limit)"] {
                let source = format!(
                    "{prefix}def f(value, limit, manager):\n    {region}\n        while True:\n{suspension}            tick()\n            if {condition}:\n                report()\n{cleanup}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "protected_loop_tail_chained_threaded_owners.py",
                );
            }
        }
    }
}
