async def f():
    client = None
    try:
        plan = await desired()
        client = await connect()
        handle = client.handle(plan)
        try:
            await handle.describe()
        except RPCError as error:
            if error.status != NOT_FOUND:
                raise
            try:
                await client.create(plan)
                log("created")
                return
            except AlreadyRunning:
                handle = client.handle(plan)
        await handle.update(plan)
        log("updated")
    except Exception:
        log("failed")
    finally:
        if client is not None:
            await client.close()
