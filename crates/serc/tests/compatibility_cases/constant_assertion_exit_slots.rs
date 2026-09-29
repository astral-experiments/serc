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

#[test]
fn matches_cpython_folded_assertion_success_inside_synchronous_manager() {
    for assertion in ["True", "1"] {
        for alternate in ["assert False, 'no'", "recover()"] {
            let body = format!(
                "with manager():\n    if not flag:\n        assert {assertion}, 'yes'\n    else:\n        {alternate}\n"
            );
            for scope in ["", "def f():\n", "class Scope:\n"] {
                let source = if scope.is_empty() {
                    body.clone()
                } else {
                    format!("{scope}{}", common::indent(&body))
                };
                common::assert_matches_cpython_marshal(&source, "manager_assertion_success.py");
            }
        }
    }
}

#[test]
fn preserves_folded_assertion_success_borrowing_and_return_order() {
    for condition in [
        "value and True",
        "value in items and 'ready'",
        "not (value or False)",
    ] {
        for tail in ["return value", "return None", "return consume(value)"] {
            let source =
                format!("def validate(value, items):\n    assert {condition}\n    {tail}\n");
            common::assert_matches_cpython_marshal(&source, "folded_assertion_success_return.py");
        }
    }
}
