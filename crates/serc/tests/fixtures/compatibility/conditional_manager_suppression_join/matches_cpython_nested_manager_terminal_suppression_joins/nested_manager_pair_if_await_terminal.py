async def run(outer, other, inner, request, first, second):
    with (
        outer(),
        other(),
    ):
        if first:
            if second is None:
                with inner() as value:
                    prepare(value)
                    return await request()
            else:
                return await request()
        else:
            return await request()
