async def f(flag):
    try:
        if flag:
            return await read()
    except Error:
        return await recover()
    sink(1)
    sink(2)
    raise Missing(flag)
