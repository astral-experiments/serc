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
