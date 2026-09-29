async def f(manager, work, value):
    while True:
        try:
            processed = False
            while True:
                try:
                    with manager as item:
                        if item is None:
                            break
                        processed = True
                        await work(item)
                except Error as exc:
                    if isinstance(exc, Stop):
                        raise
                    await backoff(value)
                    break
            if not processed:
                await backoff(value)
        except Stop:
            raise
        except Error:
            await backoff(value)
