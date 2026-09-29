async def f(manager, work, reply, release):
    with manager:
        owned = True
        def read_owned():
            return owned
        try:
            try:
                await work()
            except ValueError:
                await reply(None)
                return
        finally:
            if owned:
                release()
