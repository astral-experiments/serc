async def convert(outer, inner, choose, value):
    with outer:
        if choose:
            async with inner:
                return value
        else:
            return value
