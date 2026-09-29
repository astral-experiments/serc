async def consume(read, convert, first, second, pause, record):
    while True:
        try:
            value = read()
            if value is None:
                return
            result, selected = await convert(value)
            if result is not None and selected:
                record(result)
                first(result)
            elif result is not None and not selected:
                second(result)
        except:
            await pause()
            continue
