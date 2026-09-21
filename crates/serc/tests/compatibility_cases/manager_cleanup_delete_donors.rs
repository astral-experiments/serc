use crate::common::{assert_matches_cpython_marshal, indent};

#[test]
fn matches_cpython_manager_cleanup_assignment_and_delete_donors() {
    for action in [
        "value = values[0]",
        "value, other = values",
        "values.item = first",
        "del first",
        "del values.item",
        "del values[0]",
        "del (values[0], first)",
        "del ((), values.item)",
    ] {
        for body in [
            format!("if first:\n    {action}\nelse:\n    raise Error\n"),
            format!(
                "if first:\n    return values\nelif second:\n    raise Error\nelse:\n    {action}\n"
            ),
            // Two live arms keep the manager's original POP_BLOCK unlocated.
            format!("if first:\n    work()\nelif second:\n    raise Error\nelse:\n    {action}\n"),
            format!(
                "try:\n    work()\nfinally:\n    if first:\n        work()\n    else:\n        {action}\n"
            ),
        ] {
            let source = format!(
                "def f(manager, first, second, values):\n    with manager:\n{}    return values\n",
                indent(&indent(&body))
            );
            assert_matches_cpython_marshal(&source, "manager_cleanup_donors.py");
        }
    }
}

#[test]
fn matches_cpython_manager_finalizer_delete_after_else() {
    for function in ["def", "async def"] {
        let source = format!(
            "{function} f(manager, values):\n    with manager:\n        try:\n            value = values[0]\n        except Error:\n            raise\n        else:\n            values[1] = value\n        finally:\n            cleanup()\n            del values[0]\n    return values\n"
        );
        assert_matches_cpython_marshal(&source, "manager_finalizer_delete_else.py");
    }
}
