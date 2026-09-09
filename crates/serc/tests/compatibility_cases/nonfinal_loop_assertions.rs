use crate::common;

#[test]
fn matches_cpython_protected_nonfinal_loop_assertions() {
    for header in ["for value in items:", "while ready():"] {
        for message in ["", ", explain(value)"] {
            let branch = format!(
                "        {header}\n            if choose:\n                assert value < limit{message}\n            else:\n                consume(value)\n"
            );
            for protected in [
                format!("    try:\n{branch}    except Exception:\n        recover()\n"),
                format!("    with manager():\n{branch}"),
            ] {
                let source =
                    format!("def f(items, value, choose, limit):\n{protected}    finish()\n");
                common::assert_matches_cpython_marshal(&source, "protected_loop_assertions.py");
            }
        }
    }
}

#[test]
fn matches_cpython_suspended_nonfinal_loop_assertions() {
    for (declaration, suspension) in [
        ("async def", ""),
        ("def", "    yield None\n"),
        ("async def", "    yield None\n"),
    ] {
        for header in ["for value in items:", "while ready():"] {
            for condition in ["check(value)", "value is None"] {
                let source = format!(
                    "{declaration} f(items, value, choose):\n{suspension}    {header}\n        if choose:\n            assert {condition}\n        else:\n            consume(value)\n    finish()\n"
                );
                common::assert_matches_cpython_marshal(&source, "suspended_loop_assertions.py");
            }
        }
    }
}
