async def reconnect():
    while running():
        try:
            await work()
        except Error:
            recover()
        finally:
            if connected():
                await close()
                state.connected = False
        if running():
            await pause()
