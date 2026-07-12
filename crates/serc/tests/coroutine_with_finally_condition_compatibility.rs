#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_coroutine_with_finally_condition_owners() {
    let source = r#"async def membership_cleanup(manager, entrypoint, load, remove):
    with manager:
        try:
            value = await load(entrypoint)
        finally:
            if ":" not in entrypoint:
                remove(entrypoint)
    return value

async def prefect_shape(manager, storage, entrypoint, load, remove):
    with manager:
        storage.prepare()
        await storage.pull_code()
        try:
            value = await load(entrypoint)
        finally:
            if ":" not in entrypoint:
                remove(storage.destination)
    return value

async def equality_cleanup(manager, expected, actual, load, cleanup):
    with manager:
        try:
            value = await load(actual)
        finally:
            if actual != expected:
                cleanup(actual)
    return value

async def none_cleanup(manager, resource, load):
    with manager:
        try:
            value = await load(resource)
        finally:
            if resource is not None:
                resource.close()
    return value

async def two_call_cleanup(manager, entrypoint, load, remove, notify):
    with manager:
        try:
            value = await load(entrypoint)
        finally:
            if ":" not in entrypoint:
                remove(entrypoint)
                notify(entrypoint)
    return value
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_with_finally_condition_owners.py");
}
