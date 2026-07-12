#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_coroutine_with_finally_cast_await_owners() {
    let source = r#"async def cast_cleanup(manager, entrypoint, load, remove, cast):
    with manager:
        try:
            value = cast("Flow[..., Any]", await load(entrypoint))
        finally:
            if ":" not in entrypoint:
                remove(entrypoint)
    return value

async def prefect_shape(manager, storage, entrypoint, load, create_call, remove, cast):
    with manager:
        storage.prepare()
        await storage.pull_code()
        try:
            value = cast(
                "Flow[..., Any]",
                await load(create_call(entrypoint)),
            )
        finally:
            if ":" not in entrypoint:
                remove(storage.destination)
    return value

async def equality_cleanup(manager, actual, expected, load, cleanup, cast):
    with manager:
        try:
            value = cast("T", await load(actual))
        finally:
            if actual != expected:
                cleanup(actual)
    return value

async def two_call_cleanup(manager, entrypoint, load, remove, notify, cast):
    with manager:
        try:
            value = cast("T", await load(entrypoint))
        finally:
            if ":" not in entrypoint:
                remove(entrypoint)
                notify(entrypoint)
    return value
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_with_finally_cast_await_owners.py");
}
