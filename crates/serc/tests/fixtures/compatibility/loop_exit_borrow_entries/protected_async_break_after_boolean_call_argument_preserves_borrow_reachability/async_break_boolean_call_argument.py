async def f(stream, value, should_stop, should_log, should_retry, consume):
    try:
        async for item in stream:
            if should_stop(item):
                if should_log(item):
                    consume(item)
                    if value:
                        consume(value or 0)
                if should_retry(item):
                    value = item
                    continue
                value = item
                break
        consume(value)
        consume(value)
    except RuntimeError:
        consume(value)
