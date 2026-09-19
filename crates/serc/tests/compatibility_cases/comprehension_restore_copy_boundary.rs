use crate::common;

#[test]
fn matches_cpython_comprehension_restore_small_exit() {
    for (filename, source) in [
        (
            "returned_dict_two_restores.py",
            r"def build(items):
    return {name: count for name, count in items if count}
",
        ),
        (
            "coroutine_returned_dict_two_restores.py",
            r"async def build(items):
    return {name: count for name, count in items if count}
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_comprehension_restore_copy_limits() {
    for (filename, source) in [
        (
            "returned_dict_three_restores.py",
            r"async def build(items):
    return {name: count for name, count, enabled in items if enabled}
",
        ),
        (
            "returned_dict_protected_restore.py",
            r"async def build(items, cleanup):
    try:
        return {name: count for name, count in items if count}
    finally:
        cleanup()
",
        ),
        (
            "returned_dict_separate_return_line.py",
            r"async def build(items):
    return (
        {name: count for name, count in items if count}
    )
",
        ),
        (
            "dict_restore_has_continuation.py",
            r"async def build(items, finish):
    result = {name: count for name, count in items if count}
    finish(result)
    return result
",
        ),
        (
            "returned_dict_cell_restore.py",
            r"async def build(items):
    return {name: (lambda: count) for name, count in items}
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
