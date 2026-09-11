use crate::common;

#[test]
fn for_iter_original_cleanup_matches_cpython() {
    let cases = [
        (
            "for_try_return_cleanup.py",
            r"def f(items, recover):
    for item in items:
        try:
            return item
        except Exception:
            recover()
    raise RuntimeError(f'{items}')
",
        ),
        (
            "for_async_manager_return_cleanup.py",
            r"async def f(items, manager, recover):
    last = None
    for item in items:
        try:
            async with manager:
                return item
        except Exception as error:
            last = error
            await recover()
    raise RuntimeError(f'{items}: {last}')
",
        ),
        (
            "for_try_return_else_cleanup.py",
            r"def f(items, recover):
    for item in items:
        try:
            return item
        except Exception:
            recover()
    else:
        return f'{items}'
",
        ),
        (
            "cold_for_try_return_cleanup.py",
            r"def f(items, run, recover):
    try:
        run()
    except Exception:
        for item in items:
            try:
                return item
            except ValueError:
                recover()
        raise RuntimeError(f'{items}')
",
        ),
        (
            "for_cleanup_followed_by_folded_condition.py",
            r"def f(items, recover):
    for item in items:
        try:
            return item
        except Exception:
            recover()
    if False:
        recover()
    return f'{items}'
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
