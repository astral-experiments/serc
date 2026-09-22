use crate::common;

#[test]
fn matches_cpython_nested_or_sequence_pop_locations() {
    for (filename, source) in [
        (
            "pattern_nested_sequence_or.py",
            r"def f(subject):
    match subject:
        case Box(
            args=[int(value) | str(value), *_],
        ):
            return value
    return None
",
        ),
        (
            "pattern_nested_sequence_or_guard.py",
            r"def f(subject, allowed):
    match subject:
        case Box(
            args=[int(value) | str(value), *_],
        ) if value in allowed:
            return value
    return None
",
        ),
        (
            "pattern_distinct_line_control.py",
            r"def f(subject):
    match subject:
        case Box(args=[
            int(value) | str(value), *_
        ]):
            return value
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
