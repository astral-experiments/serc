async def f(source, open_result, stop, close, idle):
    while True:
        result = None
        try:
            result = await open_result()
            async for item in source:
                if stop(item):
                    return
        except Exception as error:
            record(error)
        finally:
            if result is not None:
                close(result)
        await idle()
