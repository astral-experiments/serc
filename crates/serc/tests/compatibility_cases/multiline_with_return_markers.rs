use crate::common;

#[test]
fn matches_cpython_multiline_return_markers_before_with_cleanup() {
    for (function, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        for value in ["value", "value.make()\n            .finish()"] {
            let source = format!(
                "{function} f(value):\n    {manager} manager():\n        return (\n            {value}\n        )\n"
            );
            common::assert_matches_cpython_marshal(&source, "multiline_with_return.py");
        }
    }
}

#[test]
fn matches_cpython_same_line_multiline_and_return_in_manager() {
    let source = r"def respond(manager, retained, response, completed):
    with manager:
        if completed:
            return retained is not None and (
                retained.status == response.status
                and retained.read() == response.body
            )
        return True
";
    common::assert_matches_cpython_marshal(
        source,
        "same_line_multiline_and_manager_return_target.py",
    );
}
