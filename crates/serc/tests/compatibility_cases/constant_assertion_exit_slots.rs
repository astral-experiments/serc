use crate::common;

#[test]
fn matches_cpython_constant_assertion_after_manager_cleanup() {
    for (scope, manager, body) in [
        ("", "with", "value = build()"),
        ("", "with", "yield value"),
        ("async ", "async with", "value = await build()"),
        ("async ", "async with", "yield value"),
    ] {
        for tail in [
            "assert True",
            "pass\n    assert True",
            "assert True\n    pass",
            "pass\n    assert 1, 'ready'\n    pass",
            "assert True\n    assert True",
        ] {
            let source = format!(
                "{scope}def collect():\n    {manager} manager():\n        try:\n            {body}\n        finally:\n            close()\n    {tail}\n"
            );
            common::assert_matches_cpython_marshal(&source, "constant_assertion_exit.py");
        }
    }
}
