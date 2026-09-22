use crate::common;

#[test]
fn matches_cpython_multiline_match_swap_nops() {
    for source in [
        r"def f(value):
    match value:
        case C(
            first=A(x=a),
            second=B(item=[_]) as b,
        ):
            return a, b
",
        r"def f(value):
    match value:
        case C(
            values=[C(items=[_, a]), b]):
            return a, b
",
        r"def f(value):
    match value:
        case C(
            first=a,
            second=B() | D() as b):
            return a, b
",
        r"def f(value):
    match value:
        case A(a, b, c) | B(
            A(a, b, c)
        ):
            return a, b, c
",
        r"def f(value):
    try:
        match value:
            case C(
                first=A(x=a),
                second=B(item=[_]) as b,
            ):
                return a, b
    except Exception:
        return value
",
        r"def f(value):
    try:
        match value:
            case C(
                first=a,
                second=B() | D() as b):
                return a, b
    except Exception:
        return value
",
        r"def f(value):
    match value:
        case C(
            values=[C(items=[_, a]), b]) if value:
            return a, b
",
        r"def f(value):
    match value:
        case C(
            first=a,
            second=b,
            third=c,
        ):
            return a, b, c
",
        r"def f(value):
    match value:
        case C(
            first=A(x=a),
            second=B(item=[_]) as b,
        ):
            pass
            return a, b
",
    ] {
        common::assert_matches_cpython_marshal(source, "multiline_match_swap_nops.py");
    }
}
