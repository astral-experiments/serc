async def probe(work, a, b, flag):
    try:
        await work()
    except ValueError:
        await work()
        if flag:
            value = await a()
            b(value)
        else:
            if flag:
                yield 1
            value = await a()
    else:
        b(0)
    b(1)
