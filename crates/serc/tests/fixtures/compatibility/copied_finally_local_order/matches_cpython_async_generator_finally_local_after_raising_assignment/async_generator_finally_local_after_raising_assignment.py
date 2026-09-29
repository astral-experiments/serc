async def f(stream):
    try:
        start = timer()
        async for item in stream:
            yield item
    except TimeoutError as error:
        report()
        raise error
    finally:
        elapsed = timer() - start
        cleanup(elapsed)
