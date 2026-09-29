async def f(values, flag):
    ready = False
    try:
        try:
            async for value in values:
                if flag:
                    continue
                yield value
            if flag:
                yield 1
            try:
                await work()
                ready = True
            except Error as error:
                await recover(error)
                ready = True
                raise
            return
        except OtherError as error:
            await recover(error)
            ready = True
            if flag:
                raise
            consume(error)
        async for value in values:
            yield value
    except Error as error:
        if not ready:
            await recover(error)
        raise
