async def consume(read, convert, first, second, pause, record):
    while True:
        try:
            value = read()
            if value is None:
                return
            result, selected = await convert(value)
            if result is not None and selected:
                with first() as stream:
                    record(result)
                    stream.write(result)
            elif result is not None and not selected:
                with second() as stream:
                    stream.write(result)
        except:
            await pause()
            continue
