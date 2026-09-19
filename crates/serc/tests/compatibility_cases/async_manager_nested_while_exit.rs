use crate::common;

#[test]
fn matches_cpython_async_manager_nested_while_exit() {
    common::assert_matches_cpython_marshal(
        r#"async def update(manager, cache, enabled, limit, load):
    result = await load()
    async with manager:
        if enabled:
            cache["current"] = result
            while len(cache) > limit:
                cache.popitem(last=False)
    return result
"#,
        "async_manager_nested_while_exit.py",
    );
}

#[test]
fn matches_cpython_async_manager_direct_while_exit_control() {
    common::assert_matches_cpython_marshal(
        r#"async def update(manager, cache, limit, load):
    result = await load()
    async with manager:
        cache["current"] = result
        while len(cache) > limit:
            cache.popitem(last=False)
    return result
"#,
        "async_manager_direct_while_exit_control.py",
    );
}

#[test]
fn matches_cpython_sync_manager_nested_while_exit_control() {
    common::assert_matches_cpython_marshal(
        r#"async def update(manager, cache, enabled, limit, load):
    result = await load()
    with manager:
        if enabled:
            cache["current"] = result
            while len(cache) > limit:
                cache.popitem(last=False)
    return result
"#,
        "sync_manager_nested_while_exit_control.py",
    );
}

#[test]
fn matches_cpython_async_manager_protected_while_exit_control() {
    common::assert_matches_cpython_marshal(
        r"async def update(manager, cache, enabled, limit, recover):
    async with manager:
        if enabled:
            try:
                while len(cache) > limit:
                    cache.popitem(last=False)
            except LookupError:
                recover()
    return cache
",
        "async_manager_protected_while_exit_control.py",
    );
}
