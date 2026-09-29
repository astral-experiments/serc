async def f(sink):
    try:
        while True:
            item = await receive()
            if item is None:
                break
            consume(item)
    except Error as error:
        report(error)
    finally:
        try:
            await sink.close()
        except Timeout:
            pass
