async def probe(flag, value):
    try:
        value = await read(value)
        if flag:
            return
    except Error as error:
        if error.kind:
            raise
        await recover(value)
    consume(value)
    consume(flag)
    return value
