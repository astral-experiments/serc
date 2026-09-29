use crate::common;

#[test]
fn matches_cpython_condition_after_async_comprehension() {
    for (filename, condition) in [
        ("false_after_async_comprehension.py", "len(items) > 10"),
        ("true_after_async_comprehension.py", "not items"),
        ("not_none_after_async_comprehension.py", "items is None"),
        ("none_after_async_comprehension.py", "items is not None"),
    ] {
        let source = format!(
            r"async def f(source, enabled, finish):
    if enabled:
        items = [item async for item in source]
        if {condition}:
            raise ValueError
    finish(source)
    return source
"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}
