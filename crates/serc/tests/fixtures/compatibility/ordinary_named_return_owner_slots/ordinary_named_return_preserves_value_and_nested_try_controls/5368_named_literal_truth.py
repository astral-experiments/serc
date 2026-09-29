async def f(client):
    try:
        try:
            await work()
        except Error as error:
            log(error)
            return 7
    except OuterError:
        log("outer")
    finally:
        if client:
            await client.close()
