async def f(stream, values, value, should_stop, should_finish, consume):
    try:
        async for item in stream:
            consume(item)
            if should_stop(item):
                value = item
                for inner in values:
                    consume(inner)
                if should_finish(item):
                    value = item
                    break
        consume(value)
        consume(value)
    finally:
        consume(value)
