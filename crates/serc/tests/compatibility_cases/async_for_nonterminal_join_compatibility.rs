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
