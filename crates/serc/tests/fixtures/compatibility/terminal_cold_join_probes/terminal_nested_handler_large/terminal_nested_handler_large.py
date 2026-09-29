async def probe(flag, value):
    try:
        value = await read(value)
        if flag:
            return
    except Error as error:
        if error.kind:
            raise
        try:
            await recover(value)
        except OtherError as nested:
            if nested.kind:
                raise
    consume(value)
    consume(flag)
    return value
