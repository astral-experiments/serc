async def run(flag, work, cleanup):
    try:
        if flag:
            try:
                await work()
            finally:
                cleanup()
        else:
            await work()
    except RuntimeError:
        return Result.FAILED
    return Result.READY
