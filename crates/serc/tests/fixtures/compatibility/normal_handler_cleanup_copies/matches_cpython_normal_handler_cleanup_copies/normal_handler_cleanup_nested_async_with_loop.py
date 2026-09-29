async def probe(manager, work, a, b, flag, inner, items):
    async with manager:
        try:
            await work()
        except ValueError:
            value = 1
            if flag:
                value = 2
                if inner:
                    a()
                else:
                    values = []
                    for item in items:
                        values.append(item)
                    b(values)
