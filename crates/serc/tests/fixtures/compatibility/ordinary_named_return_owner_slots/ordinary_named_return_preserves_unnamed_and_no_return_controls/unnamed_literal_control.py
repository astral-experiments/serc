async def f(client):
    try:
        try:
            await work()
        except Error:
            log("error")
            return 7
    except OuterError:
        log("outer")
    finally:
        if client:
            await client.close()
