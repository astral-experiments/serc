async def f(factory, suggestion, replacement):
    try:
        async with factory() as stream:
            async for item in stream:
                yield item
    except LookupError as error:
        if is_limit(error):
            record("error", error, replacement)
            yield Event(detail=Message(text="failed"))
            return
        raise
    finally:
        suggestion = replacement or suggestion
