async def f(manager, items, engine):
    with manager:
        with manager:
            for index in items:
                result = []
                if index == 0:
                    value = engine.read()
                    result = [value]
                elif index == 1:
                    value = engine.read()
                    result = list(value)
                elif index == 2:
                    value = await engine.read()
                    result = [value]
                else:
                    value = await engine.read()
                    result = [item async for item in value]
                text = "".join(item[0] for item in result)
                assert text == "value"
