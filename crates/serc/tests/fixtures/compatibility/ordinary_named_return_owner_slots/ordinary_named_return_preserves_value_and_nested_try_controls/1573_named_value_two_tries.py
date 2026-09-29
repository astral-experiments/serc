async def f(client):
    try:
        try:
            await work()
        except Error as error:
            try:
                try:
                    return result(error)
                except InnerError:
                    log("inner")
            except RecoveryError:
                log("recovery")
    except OuterError:
        log("outer")
    finally:
        if client:
            await client.close()
