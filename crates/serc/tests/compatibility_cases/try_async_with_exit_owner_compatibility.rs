use crate::common;

#[test]
fn matches_cpython_marshal_for_try_async_with_exit_owners() {
    let source = r#"async def task_group_finally(manager, nested, start, cleanup):
    try:
        async with manager as group:
            with nested(group):
                start(group)
    finally:
        cleanup(group)
        cleanup(manager)


async def connection_worker(manager, event, sleep, errors, success):
    try:
        async with manager:
            if event:
                event.set()
            await sleep()
    except PoolError as error:
        errors.append(error)
    else:
        success.append(True)


async def bound_connection_worker(manager, clock, errors, results):
    start = clock()
    try:
        async with manager as connection:
            await connection.execute()
            value = connection.info.value
    except PoolError as error:
        errors.append((clock() - start, error))
    else:
        results.append((clock() - start, value))


async def cleanup_manager(manager, logger, setup):
    handler = setup(logger)
    try:
        async with manager as pool:
            await pool.wait(timeout=2)
    finally:
        logger.remove(handler)
        logger.restore()


async def nested_scope_with_handlers(scope, manager, pause, fail):
    with scope:
        try:
            async with manager as group:
                group.start()
                await pause()
                scope.cancel()
                await pause()
        except BaseException as error:
            if fail(error):
                raise
            fail(error)
        else:
            fail(None)


async def configuration_step(manager, make_client, connect, errors, complete, value):
    if value is not None:
        try:
            async with manager:
                client = make_client(value)
                await connect(client)
        except ApiError, ClientError, TimeoutError:
            errors["base"] = "cannot_connect"
        except InvalidKeyError:
            errors["base"] = "invalid_key"
        else:
            return complete(client)
    return errors


async def two_awaits_under_timeout(manager, session, payload, use):
    try:
        async with manager:
            response = await session.post(payload)
            result = await response.json()
        if response.failed or "error" in result:
            use(result)
    except TimeoutError:
        use(payload)


async def plain_async_with_control(manager, pause, use):
    async with manager:
        await pause()
    use(manager)


async def synchronous_with_in_try_control(manager, use):
    try:
        with manager:
            use(manager)
    except ValueError:
        use(None)


async def post_async_with_try_control(manager, pause, use):
    async with manager:
        await pause()
    try:
        use(manager)
    except ValueError:
        use(None)
"#;

    common::assert_matches_cpython_marshal(source, "try_async_with_exit_owners.py");
}
