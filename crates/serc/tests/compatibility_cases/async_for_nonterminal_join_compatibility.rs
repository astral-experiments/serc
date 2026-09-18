use crate::common;

#[test]
fn async_for_nonterminal_join_target() {
    let source = r#"async def upload_parts(primary, secondary, client, retry, destination):
    first_payload = primary
    second_payload = secondary

    if first_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    first_payload.encode(),
                    f"{destination}/first.txt",
                )
    else:
        raise ValueError("First payload is missing")
    if second_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    second_payload.encode(),
                    f"{destination}/second.txt",
                )
    else:
        raise ValueError("Second payload is missing")

    async for attempt in retry(stop=2, wait=1, reraise=True):
        with attempt:
            await client.finish(destination)
    result = await client.result()
    return result
"#;
    common::assert_matches_cpython_marshal(source, "async_for_nonterminal_join_target.py");
}

#[test]
fn async_for_nonterminal_join_no_first_else_control() {
    let source = r#"async def upload_parts(primary, secondary, client, retry, destination):
    first_payload = primary
    second_payload = secondary

    if first_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    first_payload.encode(),
                    f"{destination}/first.txt",
                )
    if second_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    second_payload.encode(),
                    f"{destination}/second.txt",
                )
    else:
        raise ValueError("Second payload is missing")

    async for attempt in retry(stop=2, wait=1, reraise=True):
        with attempt:
            await client.finish(destination)
    result = await client.result()
    return result
"#;
    common::assert_matches_cpython_marshal(source, "async_for_nonterminal_join_no_first_else.py");
}

#[test]
fn async_for_nonterminal_join_first_else_falls_through_control() {
    let source = r#"async def upload_parts(primary, secondary, client, retry, destination):
    first_payload = primary
    second_payload = secondary

    if first_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    first_payload.encode(),
                    f"{destination}/first.txt",
                )
    else:
        client.missing("First payload is missing")
    if second_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    second_payload.encode(),
                    f"{destination}/second.txt",
                )
    else:
        raise ValueError("Second payload is missing")

    async for attempt in retry(stop=2, wait=1, reraise=True):
        with attempt:
            await client.finish(destination)
    result = await client.result()
    return result
"#;
    common::assert_matches_cpython_marshal(
        source,
        "async_for_nonterminal_join_else_falls_through.py",
    );
}

#[test]
fn async_for_nonterminal_join_no_raising_else_control() {
    let source = r#"async def upload_parts(primary, secondary, client, retry, destination):
    first_payload = primary
    second_payload = secondary

    if first_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    first_payload.encode(),
                    f"{destination}/first.txt",
                )
    if second_payload is not None:
        async for attempt in retry(
            stop=2,
            wait=1,
            reraise=True,
        ):
            with attempt:
                await client.upload(
                    second_payload.encode(),
                    f"{destination}/second.txt",
                )

    async for attempt in retry(stop=2, wait=1, reraise=True):
        with attempt:
            await client.finish(destination)
    result = await client.result()
    return result
"#;
    common::assert_matches_cpython_marshal(source, "async_for_nonterminal_join_no_raising_else.py");
}

#[test]
fn async_for_branch_exits_preserve_copied_return_nops() {
    for (filename, source) in [
        (
            "async_for_if_arm_literal_return_nop_target.py",
            r"async def target(items, enabled):
    if enabled:
        async for item in items:
            consume(item)
    else:
        consume(None)
    return None
",
        ),
        (
            "async_for_if_arm_local_return_nop_target.py",
            r"async def target(items, enabled, value):
    if enabled:
        value = None
        async for item in items:
            value = transform(item)
    else:
        value = transform(None)
    return value
",
        ),
        (
            "sync_for_if_arm_local_return_nop_control.py",
            r"async def target(items, enabled, value):
    if enabled:
        value = None
        for item in items:
            value = transform(item)
    else:
        value = transform(None)
    return value
",
        ),
        (
            "async_for_else_if_arm_return_nop_control.py",
            r"async def target(items, enabled, value):
    if enabled:
        async for item in items:
            value = transform(item)
        else:
            finish()
    else:
        value = transform(None)
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn async_for_finally_bare_returns_keep_join_nops() {
    for (filename, source) in [
        (
            "async_generator_finally_async_for_bare_return_target.py",
            r"async def target(enabled, items):
    if enabled:
        try:
            try:
                async for item in items:
                    pass
            except Exception:
                pass
            yield make_value()
        finally:
            cleanup = items
            async for item in cleanup:
                pass
        return
    async for item in items:
        yield item
",
        ),
        (
            "coroutine_finally_async_for_bare_return_target.py",
            r"async def target(enabled, items):
    if enabled:
        try:
            await work()
        finally:
            cleanup = items
            async for item in cleanup:
                consume(item)
        return
    await finish()
",
        ),
        (
            "async_generator_finally_async_for_following_call_control.py",
            r"async def target(enabled, items):
    if enabled:
        try:
            yield make_value()
        finally:
            async for item in items:
                consume(item)
            finish()
        return
    yield None
",
        ),
        (
            "async_generator_finally_async_for_else_return_control.py",
            r"async def target(enabled, items):
    if enabled:
        try:
            yield make_value()
        finally:
            async for item in items:
                consume(item)
            else:
                finish()
        return
    yield None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
