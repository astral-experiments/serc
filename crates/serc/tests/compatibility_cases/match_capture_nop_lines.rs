use crate::common;

#[test]
fn matches_cpython_kw2_same_line() {
    let source = r"def f(subject):
    match subject:
        case C(a=a, b=b):
            return make(a, b)
        case _:
            return fallback(subject)
";
    common::assert_matches_cpython_marshal(source, "match_capture_kw2_same_line.py");
}

#[test]
fn matches_cpython_kw2_multiline() {
    let source = r"def f(subject):
    match subject:
        case C(
            a=a, b=b
        ):
            return make(a, b)
        case _:
            return fallback(subject)
";
    common::assert_matches_cpython_marshal(source, "match_capture_kw2_multiline.py");
}

#[test]
fn matches_cpython_kw3_split_capture_lines() {
    let source = r"def f(subject):
    match subject:
        case C(
            a=a,
            b=b,
            c=c
        ):
            return make(a, b, c)
        case _:
            return fallback(subject)
";
    common::assert_matches_cpython_marshal(source, "match_capture_kw3_split_capture_lines.py");
}

#[test]
fn matches_cpython_kw3_assign_body() {
    let source = r"def f(subject):
    match subject:
        case C(
            a=a, b=b, c=c
        ):
            result = make(a, b, c)
        case _:
            result = fallback(subject)
    return result
";
    common::assert_matches_cpython_marshal(source, "match_capture_kw3_assign_body.py");
}

#[test]
fn drops_constant_true_guard_line_after_same_line_capture() {
    for (filename, source) in [
        (
            "match_capture_true_guard.py",
            r"def f(value):
    match value:
        case [item] if True:
            pass
        case _:
            pass
",
        ),
        (
            "match_capture_copied_true_guard.py",
            r"def f(value):
    match value:
        case [item] if True:
            pass
        case [other] if ready(other):
            pass
        case _:
            pass
",
        ),
        (
            "match_capture_true_guard_return.py",
            r"def f(value):
    match value:
        case Thing(item) if 1:
            return item
        case _:
            return None
",
        ),
        (
            "match_capture_multiline_pattern_guard.py",
            r"def f(value):
    match value:
        case [
            item,
        ] if True:
            pass
        case _:
            pass
",
        ),
        (
            "match_capture_multiline_true_guard.py",
            r"def f(value):
    match value:
        case [item] if (
            True
        ):
            pass
        case _:
            pass
",
        ),
        (
            "match_capture_runtime_guard.py",
            r"def f(value):
    match value:
        case [item] if ready(item):
            pass
        case _:
            pass
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn drops_redundant_constant_guard_line_after_refutable_pattern() {
    for (filename, source) in [
        (
            "match_mapping_true_guard_line.py",
            "def f(value):\n    match value:\n        case {} if True:\n            pass\n",
        ),
        (
            "match_wildcard_sequence_true_guard_line.py",
            "def f(value):\n    match value:\n        case [_, *_, _] if 1:\n            pass\n",
        ),
        (
            "match_or_false_guard_line.py",
            "def f(value):\n    match value:\n        case 1 | 2 if False:\n            return 1\n        case _:\n            return 2\n",
        ),
        (
            "match_mapping_separate_true_guard_line.py",
            "def f(value):\n    match value:\n        case {} if (\n            True\n        ):\n            pass\n",
        ),
        (
            "match_or_separate_false_guard_line.py",
            "def f(value):\n    match value:\n        case 1 | 2 if (\n            False\n        ):\n            return 1\n        case _:\n            return 2\n",
        ),
        (
            "match_mapping_runtime_guard_line.py",
            "def f(value, condition):\n    match value:\n        case {} if condition:\n            pass\n",
        ),
        (
            "match_value_true_guard_line.py",
            "def f(value):\n    match value:\n        case 1 if True:\n            pass\n",
        ),
        (
            "match_singleton_false_guard_line.py",
            "def f(value):\n    match value:\n        case None if False:\n            return 1\n        case _:\n            return 2\n",
        ),
        (
            "match_mapping_false_guard_line.py",
            "def f(value):\n    match value:\n        case {} if False:\n            return 1\n        case _:\n            return 2\n",
        ),
        (
            "match_or_false_guard_without_default.py",
            "def f(value):\n    match value:\n        case 1 | 2 if False:\n            return 1\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn constant_false_guard_falls_through_to_following_statement() {
    for pattern in [
        "captured",
        "captured as alias",
        "1 | 2",
        "[captured] | (captured,)",
        "_",
    ] {
        for context in ["plain", "while", "for"] {
            let body =
                format!("match value:\n    case {pattern} if False:\n        pass\naction()\n");
            let body = match context {
                "while" => format!("while flag:\n{}", common::indent(&body)),
                "for" => format!("for value in values:\n{}", common::indent(&body)),
                _ => body,
            };
            let source = format!("def f(value, flag, values):\n{}", common::indent(&body));
            common::assert_matches_cpython_marshal(&source, "match_false_following.py");
        }
    }
    for source in [
        "def f(value):\n    match value:\n        case captured if False:\n            pass\n",
        "def f(value):\n    match value:\n        case captured if (\n            False\n        ):\n            pass\n    action()\n",
        "def f(value):\n    match value:\n        case _ if (\n            False\n        ):\n            pass\n    action()\n",
        "def f(value, flag):\n    match value:\n        case captured if flag:\n            pass\n    action()\n",
        "def f(value):\n    match value:\n        case 1 | 2 if False:\n            pass\n        case _:\n            action()\n",
        "def f(value):\n    match value:\n        case [item] as captured if False:\n            pass\n    action()\n",
        "def f(value):\n    match value:\n        case (1 | 2) as captured if False:\n            pass\n    action()\n",
    ] {
        common::assert_matches_cpython_marshal(source, "match_false_control.py");
    }
}
