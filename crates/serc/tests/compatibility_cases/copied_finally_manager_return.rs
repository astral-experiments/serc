use crate::common;

#[test]
fn matches_cpython_returns_after_copied_finally_managers() {
    for (definition, prologue) in [("def", ""), ("def", "    yield value\n"), ("async def", "")] {
        for returned in ["value", "work(value)"] {
            for prefix in ["", "        cleanup()\n"] {
                for body in ["pass", "consume(value)"] {
                    let source = format!(
                        "{definition} collect(value, manager):\n{prologue}    try:\n        return {returned}\n    finally:\n{prefix}        with manager:\n            {body}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_finally_manager_return.py",
                    );
                }
            }
        }
    }
}
