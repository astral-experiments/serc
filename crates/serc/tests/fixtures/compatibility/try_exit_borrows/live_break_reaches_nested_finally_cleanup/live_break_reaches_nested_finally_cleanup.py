async def f(sink, flag):
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
        finally:
            if flag:
                await sink.release()
