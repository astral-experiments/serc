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
fn matches_cpython_post_await_folded_assertion_return_target() {
    let source = r"def make_query(client):
    async def query(limit=None, filters=None, order='DESC'):
        result = await client.fetch(limit=limit, filters=filters, order=order)
        assert len(result) <= limit if limit is not None else True
        return result
    return query
";
    common::assert_matches_cpython_marshal(source, "post_await_folded_assertion_return_target.py");
}

#[test]
fn matches_cpython_post_await_boolean_assertion_return_control() {
    let source = r"def make_query(client):
    async def query(limit=None, filters=None, order='DESC'):
        result = await client.fetch(limit=limit, filters=filters, order=order)
        assert limit is None or len(result) <= limit
        return result
    return query
";
    common::assert_matches_cpython_marshal(
        source,
        "post_await_boolean_assertion_return_control.py",
    );
}

#[test]
fn matches_cpython_terminal_folded_integer_assertion_return() {
    let source = r"def check_status(client, blocked):
    if blocked:
        client.block()
    response = client.get()
    assert response.status_code == 503 if blocked else 200
";
    common::assert_matches_cpython_marshal(source, "terminal_folded_integer_assertion_return.py");
}

#[test]
fn matches_cpython_terminal_folded_true_assertion_return() {
    let source = r"def check_value(extra, value):
    assert extra(value) if extra else True
";
    common::assert_matches_cpython_marshal(source, "terminal_folded_true_assertion_return.py");
}

#[test]
fn matches_cpython_folded_condition_explicit_constant_return_target() {
    let source = r"def should_skip(path, custom=None):
    if path.endswith('/skip') or (custom(path) if custom else False):
        return True
    return False
";
    common::assert_matches_cpython_marshal(
        source,
        "folded_condition_explicit_constant_return_target.py",
    );
}

#[test]
fn matches_cpython_boolean_condition_explicit_constant_return_control() {
    let source = r"def should_skip(path, custom=None):
    if path.endswith('/skip') or (custom and custom(path)):
        return True
    return False
";
    common::assert_matches_cpython_marshal(
        source,
        "boolean_condition_explicit_constant_return_control.py",
    );
}
