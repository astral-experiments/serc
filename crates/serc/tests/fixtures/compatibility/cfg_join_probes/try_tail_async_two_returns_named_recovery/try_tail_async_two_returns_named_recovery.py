async def f(mode):
    try:
        if mode == 1:
            return await read()
        if mode == 2:
            return await consume()
    except Error as error:
        if mode:
            raise
        return await recover(error)
    raise Missing(f"unsupported mode: {mode}")
