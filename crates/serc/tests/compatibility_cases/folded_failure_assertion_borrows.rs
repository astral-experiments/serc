use std::fmt::Write as _;

use crate::common;

fn check(body: &str, assertion: &str, declaration: &str, generator: bool) {
    let mut source = format!("{declaration} function(flag, value, other, values, manager):\n");
    for line in body.lines() {
        if line.trim() == "ASSERT" {
            let indentation = &line[..line.len() - line.trim_start().len()];
            for part in assertion.lines() {
                writeln!(source, "    {indentation}{part}")
                    .expect("writing to a String cannot fail");
            }
        } else {
            writeln!(source, "    {line}").expect("writing to a String cannot fail");
        }
    }
    source.push_str(if generator {
        "    yield value\n"
    } else {
        "    return value\n"
    });
    common::assert_matches_cpython_marshal(&source, "folded_failure_assertion.py");
}

#[test]
fn preserves_empty_blocks_before_folded_assertion_messages() {
    for body in [
        "if flag:\n    ASSERT",
        "while flag:\n    ASSERT",
        "for item in values:\n    if flag:\n        ASSERT",
        "match flag:\n    case 1:\n        ASSERT\n    case _:\n        consume(value)",
        "while flag:\n    consume(value)\nASSERT",
        "with manager:\n    consume(value)\nASSERT",
    ] {
        for (declaration, generator) in [
            ("def", false),
            ("async def", false),
            ("def", true),
            ("async def", true),
        ] {
            for assertion in [
                "assert False, value",
                "assert False, f'failure: {value!r}: {other}'",
            ] {
                check(body, assertion, declaration, generator);
            }
        }
    }
}

#[test]
fn keeps_borrowing_after_nonempty_assertion_prefixes() {
    for body in [
        "ASSERT",
        "if flag:\n    consume(value)\n    ASSERT",
        "for item in values:\n    ASSERT",
        "while flag:\n    pass\n    ASSERT",
    ] {
        for (declaration, generator) in [
            ("def", false),
            ("async def", false),
            ("def", true),
            ("async def", true),
        ] {
            check(
                body,
                "assert False, f'failure: {value!r}: {other}'",
                declaration,
                generator,
            );
        }
    }
}

#[test]
fn keeps_borrowing_through_retained_false_assertion_nops() {
    for body in [
        "ASSERT",
        "if flag:\n    ASSERT",
        "if flag:\n    consume(value)\n    ASSERT",
        "for item in values:\n    ASSERT",
        "for item in values:\n    if flag:\n        ASSERT",
        "while flag:\n    ASSERT",
        "while flag:\n    pass\n    ASSERT",
        "while flag:\n    consume(value)\nASSERT",
        "match flag:\n    case 1:\n        ASSERT\n    case _:\n        consume(value)",
        "with manager:\n    consume(value)\nASSERT",
    ] {
        for condition in ["False", "0"] {
            let assertion =
                format!("assert (\n    {condition}\n), f'failure: {{value!r}}: {{other}}'");
            check(body, &assertion, "def", false);
        }
    }
}

#[test]
fn folded_assertion_empties_only_an_unmarked_match_fallback() {
    for body in [
        "assert False, value",
        "pass; assert False, value",
        "saved = value; assert False, value",
        "\n            assert False, value",
    ] {
        let source = format!(
            "def choose(value):\n    match value:\n        case Keys.first:\n            return 1\n        case Keys.second:\n            return 2\n        case _: {body}\n"
        );
        common::assert_matches_cpython_marshal(&source, "same_line_assertion.py");
    }
}

#[test]
fn retained_assertion_lines_keep_branch_targets_reachable() {
    for body in [
        "if flag:\n    return value\nelse:\n    ASSERT",
        "if flag:\n    consume(value)\nelse:\n    ASSERT",
        "if flag:\n    return value\nASSERT",
    ] {
        for assertion in [
            "assert False, message(value)",
            "assert (\n    False\n), message(value)",
        ] {
            check(body, assertion, "def", false);
        }
    }
}
