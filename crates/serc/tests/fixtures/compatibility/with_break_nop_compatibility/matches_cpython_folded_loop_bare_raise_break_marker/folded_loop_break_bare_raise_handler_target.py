async def run(work, cleanup, done, kind):
    try:
        await work()
    except BaseException as error:
        if isinstance(error, kind):
            while True:
                try:
                    await cleanup()
                    break
                except kind:
                    if done():
                        break
        else:
            await cleanup()
        raise
