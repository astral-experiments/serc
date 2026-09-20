use crate::common;
use crate::common::indent;

#[test]
fn threads_final_elif_conditions_through_match_case_exits() {
    for condition in [
        "other",
        "other is None",
        "not other",
        "first and other",
        "first or other",
    ] {
        let matched = format!(
            "match subject:\n    case 1:\n        before()\n    case 2:\n        if flag:\n            yes()\n        elif {condition}:\n            consume(subject)\n    case _:\n        last_case()\nafter()\n"
        );
        for context in [
            "function",
            "module",
            "class",
            "for",
            "while",
            "try",
            "handler",
            "with",
            "async",
            "generator",
        ] {
            let body = match context {
                "for" => format!("for subject in values:\n{}", indent(&matched)),
                "while" => format!("while active:\n{}", indent(&matched)),
                "try" => format!("try:\n{}except Error:\n    recover()\n", indent(&matched)),
                "handler" => format!("try:\n    before()\nexcept Error:\n{}", indent(&matched)),
                "with" => format!("with manager:\n{}", indent(&matched)),
                "generator" => format!("{matched}yield subject\n"),
                _ => matched.clone(),
            };
            let source = match context {
                "module" => body,
                "class" => format!("class C:\n{}", indent(&body)),
                _ => format!(
                    "{}def f(subject, flag, first, other, last, values, active, manager):\n{}",
                    if context == "async" { "async " } else { "" },
                    indent(&body),
                ),
            };
            common::assert_matches_cpython_marshal(&source, "match_elif_case_exits.py");
        }
    }
}

#[test]
fn preserves_small_return_copying_before_match_elif_threading() {
    for condition in ["other", "other is None", "first and other"] {
        for loop_body in [false, true] {
            let matched = format!(
                "match subject:\n    case 1:\n        before()\n    case 2:\n        if flag:\n            yes()\n        elif {condition}:\n            no()\n    case _:\n        last()\nreturn subject\n"
            );
            let body = if loop_body {
                format!("for subject in values:\n{}", indent(&matched))
            } else {
                matched
            };
            let source = format!(
                "def f(subject, flag, other, first, values):\n{}",
                indent(&body),
            );
            common::assert_matches_cpython_marshal(&source, "match_elif_small_return.py");
        }
    }
}

#[test]
fn preserves_protected_multi_edge_chains_at_implicit_return() {
    for handler in ["raise", "pass", "return None", "return result", "recover()"] {
        for case_body in ["result = value", "consume()"] {
            for asynchronous in [false, true] {
                let matched = format!(
                    "match subject:\n    case 1:\n        if flag:\n            {case_body}\n        elif first and other:\n            {case_body}\n    case _:\n        {case_body}\n"
                );
                let body = format!("try:\n{}except Error:\n    {handler}\n", indent(&matched));
                let source = format!(
                    "{}def f(subject, flag, first, other, value):\n{}",
                    if asynchronous { "async " } else { "" },
                    indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "protected_match_elif.py");
            }
        }
    }
}

#[test]
fn retains_original_noop_paths_before_threading_match_comparison_exits() {
    for function in ["def", "async def"] {
        for condition in [
            "first < other < last",
            "first and other",
            "first or other",
            "first if last else other",
        ] {
            for body in ["pass", "42", "1 + 2"] {
                let source = format!(
                    "{function} f(subject, flag, first, other, last):\n    try:\n        match subject:\n            case 1:\n                if flag:\n                    consume()\n                elif {condition}:\n                    {body}\n            case _:\n                fallback()\n        inside_after()\n    except Error:\n        recover()\n    after()\n"
                );
                common::assert_matches_cpython_marshal(&source, "original_match_noop_path.py");
            }
        }
    }
}
