async def f(client):
    try:
        try:
            await work()
        except Error as error:
            log(error)
            return
    except OuterError:
        log("outer")
    finally:
        if client is not None:
            await client.close()
