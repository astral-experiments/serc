async def f(stream):
    try:
        async for value in stream:
            consume(value)
    except Error as error:
        report(error)
        raise Failure
    finally:
        await finish()
        await close()
