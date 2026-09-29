async def f(stream, values, value, should_stop, should_finish, consume):
    try:
        async for item in stream:
            if should_finish(item):
                break
            consume(item)
            if should_stop(item):
                for inner in values:
                    consume(inner)
                if should_finish(value):
                    value = item
                    break
        consume(value)
        consume(value)
    finally:
        consume(value)
