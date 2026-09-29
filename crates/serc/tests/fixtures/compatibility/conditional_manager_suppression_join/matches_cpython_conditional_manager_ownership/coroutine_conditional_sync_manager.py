async def convert(outer, inner, choose, value):
    with outer:
        if choose:
            with inner:
                return value
        else:
            return value
