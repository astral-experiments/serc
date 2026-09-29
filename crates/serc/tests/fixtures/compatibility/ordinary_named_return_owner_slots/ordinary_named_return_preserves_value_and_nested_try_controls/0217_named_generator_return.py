async def f(client):
    try:
        try:
            await work()
        except Error as error:
            try:
                yield render(error)
                return
            except RecoveryError:
                log("recovery")
    except OuterError:
        log("outer")
    finally:
        if client is not None:
            await client.close()
