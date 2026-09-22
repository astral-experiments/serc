use crate::common;

#[test]
fn matches_cpython_pattern_capture_store_fusion() {
    for pattern in [
        "C(a, b, _, c)",
        "C(a, b, d, c)",
        "C(_, a, b, c)",
        "C(a, b, c, _)",
        "C(a, _, b, _, c)",
        "C(a, b, _, c, d, _, e)",
        "(a, b, _, c)",
        "C(\n            a,\n            b,\n            _,\n            c,\n        )",
    ] {
        common::assert_matches_cpython_marshal(
            &format!(
                "def f(x):\n    match x:\n        case {pattern}:\n            consume(a, b, c)\n"
            ),
            "pattern_capture_store_fusion.py",
        );
    }
}

#[test]
fn matches_cpython_pattern_capture_store_fusion_scope_boundaries() {
    for source in [
        "def f(x):\n    global a, b, c\n    match x:\n        case C(a, b, _, c):\n            consume(a, b, c)\n",
        "def f(x):\n    def inner():\n        return a, b\n    match x:\n        case C(a, b, _, c):\n            consume(a, b, c)\n",
        "def f(x):\n    try:\n        match x:\n            case C(a, b, _, c) if a:\n                consume(a, b, c)\n    except Exception:\n        recover()\n",
        "async def f(x):\n    match x:\n        case C(a, b, _, c):\n            await consume(a, b, c)\n",
    ] {
        common::assert_matches_cpython_marshal(source, "pattern_capture_store_scope.py");
    }
    for parameters in [13, 14, 17] {
        let parameters = (0..parameters)
            .map(|index| format!("p{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        common::assert_matches_cpython_marshal(
            &format!(
                "def f(x, {parameters}):\n    match x:\n        case C(a, b, _, c):\n            consume(a, b, c)\n"
            ),
            "pattern_capture_store_limit.py",
        );
    }
}
