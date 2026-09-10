use crate::common;

#[test]
fn matches_cpython_none_capture_swap_locations() {
    let cases = [
        (
            "match_none_swap_capture_none_capture.py",
            r"def f(value):
    match value:
        case (t, None, u):
            return t, u
",
        ),
        (
            "match_none_swap_capture_none_capture_guard.py",
            r"def f(value):
    match value:
        case (t, None, u) if u:
            return t, u
",
        ),
        (
            "match_none_swap_none_capture_capture.py",
            r"def f(value):
    match value:
        case (None, t, u):
            return t, u
",
        ),
        (
            "match_none_swap_capture_true_capture.py",
            r"def f(value):
    match value:
        case (t, True, u):
            return t, u
",
        ),
        (
            "match_none_swap_capture_none_two_captures.py",
            r"def f(value):
    match value:
        case (t, None, u, v):
            return t, u, v
",
        ),
        (
            "match_none_swap_one_swap.py",
            r"def f(value):
    match value:
        case (t, None):
            return value
",
        ),
        (
            "match_none_swap_multiline_capture.py",
            r"def f(value):
    match value:
        case (
            t,
            None,
            u,
        ):
            return value
",
        ),
        (
            "match_none_swap_multiline_none_only.py",
            r"def f(value):
    match value:
        case (t,
            None, u):
            return value
",
        ),
        (
            "match_none_swap_ordinary_try.py",
            r"def f(value, values):
    try:
        match value:
            case (t, None, u):
                observe(t, u)
            case _:
                observe(value)
    except Error:
        recover()
",
        ),
        (
            "match_none_swap_async_generator.py",
            r"async def f(value, values):
    yield value
    with manager():
        match value:
            case (t, None, u):
                observe(t, u)
            case _:
                observe(value)
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
