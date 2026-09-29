async def f(sink, item):
    try:
        await work(item)
    except Error as error:
        cause = error.cause or error.__cause__
        report(cause)
        if not recoverable(cause):
            raise
    finally:
        sink.record(item)
        done(item)
