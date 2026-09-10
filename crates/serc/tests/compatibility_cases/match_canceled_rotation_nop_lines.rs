use crate::common;

#[test]
fn matches_cpython_canceled_match_rotation_nop_lines() {
    let cases = [
        (
            "match_canceled_rotation_00_class_chain_multiline.py",
            r"def f(value):
    match value:
        case (
            str(a),
            str(b),
            c,
        ):
            return a, b, c
",
        ),
        (
            "match_canceled_rotation_01_class_chain_same_line.py",
            r"def f(value):
    match value:
        case (str(a), str(b), c):
            return a, b, c
",
        ),
        (
            "match_canceled_rotation_02_class_chain_two_last_lines.py",
            r"def f(value):
    match value:
        case (str(a),
              str(b), c):
            return a, b, c
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
