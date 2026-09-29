async def f(manager, work, reply, release):
    with manager:
        owned = True
        def read_owned():
            return owned
        try:
            try:
                await work()
            except ValueError as error:
                await reply(error)
                return
        finally:
            if owned:
                release()
