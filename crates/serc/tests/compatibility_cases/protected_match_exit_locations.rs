use crate::common;

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn matched(condition: &str, body: &str) -> String {
    format!(
        "match subject:\n    case 1:\n        before()\n    case 2:\n        if flag:\n            yes()\n        elif {condition}:\n            {body}\n    case _:\n        last_case()\n"
    )
}

fn function(body: &str) -> String {
    format!(
        "def f(subject, flag, first, other, last, values, active, manager):\n{}",
        indent(body)
    )
}

#[test]
fn clones_protected_match_exits_after_empty_outer_if() {
    for condition in [
        "other",
        "first and other",
        "first or other",
        "(first and other) or last",
    ] {
        for body in ["consume(subject)", "pass"] {
            let matched = matched(condition, body);
            let source = function(&format!(
                "try:\n    if active:\n{}except Error:\n    recover()\nafter()\nreturn subject\n",
                indent(&indent(&matched)),
            ));
            common::assert_matches_cpython_marshal(&source, "protected_match_exit_locations.py");
        }
    }
}

#[test]
fn keeps_original_nop_entries_when_cloning_match_exits() {
    for condition in ["first or other", "(first and other) or last"] {
        for context in ["plain", "try_follow", "with", "finally", "for"] {
            let matched = matched(condition, "pass");
            let body = match context {
                "try_follow" => format!(
                    "try:\n{}    inside_after()\nexcept Error:\n    recover()\n",
                    indent(&matched),
                ),
                "with" => format!(
                    "try:\n    with manager:\n{}except Error:\n    recover()\n",
                    indent(&indent(&matched)),
                ),
                "finally" => format!("try:\n{}finally:\n    cleanup()\n", indent(&matched)),
                "for" => format!("for subject in values:\n{}", indent(&matched)),
                _ => matched,
            };
            let source = function(&format!("{body}after()\nreturn subject\n"));
            common::assert_matches_cpython_marshal(&source, "match_original_nop_entries.py");
        }
    }
}

#[test]
fn clones_loop_tail_match_exits_per_condition() {
    for condition in ["other", "not other", "first and other", "first or other"] {
        for body in ["consume(subject)", "pass"] {
            for header in ["for subject in values:", "while active:"] {
                let source = function(&format!(
                    "{header}\n{}after()\n",
                    indent(&matched(condition, body)),
                ));
                common::assert_matches_cpython_marshal(&source, "loop_match_exit_locations.py");
            }
        }
    }
}

#[test]
fn terminal_nested_conditions_keep_copied_case_jump_locations() {
    for prefix in ["def", "async def"] {
        let source = format!(
            "{prefix} f(subject, first, second, left, right, result):\n    match subject:\n        case Before():\n            before()\n        case Selected():\n            if first:\n                prepare()\n                if left is not None:\n                    result['left'] = left\n            elif second:\n                prepare()\n                if right is not None:\n                    result['right'] = right\n            else:\n                raise ValueError\n        case _:\n            fallback()\n    finish(result)\n",
        );
        common::assert_matches_cpython_marshal(&source, "nested_case_jump_locations.py");
    }
}

#[test]
fn nested_case_jump_locations_preserve_other_shared_joins() {
    for (name, last_body) in [
        ("raise", "raise ValueError"),
        ("call", "fallback()"),
        ("return", "return None"),
    ] {
        let branch = format!(
            "if first:\n    if left is not None:\n        result['left'] = left\nelif second:\n    if right is not None:\n        result['right'] = right\nelse:\n    {last_body}\n",
        );
        let source = format!(
            "def f(first, second, left, right, result):\n{}    finish(result)\n",
            indent(&branch)
        );
        common::assert_matches_cpython_marshal(&source, &format!("nested_join_{name}.py"));
    }
}
