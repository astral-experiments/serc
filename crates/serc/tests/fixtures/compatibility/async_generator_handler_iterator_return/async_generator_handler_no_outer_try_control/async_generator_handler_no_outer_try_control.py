async def f(values):
    try:
        first()
    except FirstError:
        try:
            async for value in values:
                yield value
                if value:
                    return
        except SecondError as error:
            raise FinalError from error
