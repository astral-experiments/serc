use crate::common;

#[test]
fn matches_cpython_constant_assertion_borrow_boundaries() {
    for (scope, prefix) in [
        ("", ""),
        ("", "    yield value\n"),
        ("async ", "    await ready()\n"),
    ] {
        for condition in ["True", "f'ready'"] {
            for body in [
                format!("assert {condition}"),
                format!(
                    "match value:\n        case 0:\n            assert {condition}\n        case _:\n            alternate()"
                ),
                format!(
                    "match value:\n        case 0 | 1:\n            assert {condition}\n        case _:\n            alternate()"
                ),
            ] {
                let source =
                    format!("{scope}def collect(value):\n{prefix}    {body}\n    return value\n");
                common::assert_matches_cpython_marshal(&source, "assertion_borrow_boundary.py");
            }
        }
    }
    for condition in ["True", "f'ready'"] {
        let source = format!(
            "async def collect(value):\n    yield value\n    assert {condition}\n    consume(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "assertion_borrow_boundary.py");
    }
}

#[test]
fn matches_cpython_constant_assertion_finally_nop() {
    for (scope, prefix) in [
        ("", ""),
        ("", "    yield value\n"),
        ("async ", "    await ready()\n"),
    ] {
        for body_prefix in ["", "        pass\n"] {
            let source = format!(
                "{scope}def collect(value):\n{prefix}    try:\n        work()\n    finally:\n{body_prefix}        assert True\n    return value\n"
            );
            common::assert_matches_cpython_marshal(&source, "assertion_finally_boundary.py");
        }
    }
}
