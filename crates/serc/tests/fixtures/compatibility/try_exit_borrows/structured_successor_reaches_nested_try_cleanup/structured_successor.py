async def f(sink):
    try:
        while True:
            item = await receive()
            if item is None:
                report(item)
                break
            if selected(item):
                consume(item)
            advance(item)
    except Error:
        report_error()
    finally:
        try:
            await sink.close()
        except Timeout:
            pass
