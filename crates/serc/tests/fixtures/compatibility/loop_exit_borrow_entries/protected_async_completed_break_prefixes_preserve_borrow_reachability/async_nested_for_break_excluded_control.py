async def f(stream, values, value, should_stop, should_finish, consume):
    try:
        async for item in stream:
            if should_stop(item):
                for inner in values:
                    if should_finish(inner):
                        break
                    consume(inner)
                if should_finish(item):
                    value = item
                    break
        consume(value)
        consume(value)
    finally:
        consume(value)
