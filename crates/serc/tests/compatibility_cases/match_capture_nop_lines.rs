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
