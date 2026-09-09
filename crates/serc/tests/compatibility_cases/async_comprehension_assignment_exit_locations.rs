use crate::common;

#[test]
fn matches_cpython_async_comprehension_assignment_exit_locations() {
    for expression in [
        "[item async for item in items]",
        "{item async for item in items}",
        "{item: item async for item in items}",
    ] {
        let source = format!(
            "async def f(items):\n    try:\n        values = {expression}\n    except Error:\n        recover()\n    finish(values)\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "async_comprehension_assignment_exit_locations.py",
        );
    }
}
