async def f(flag, manager):
    try:
        if flag:
            await work()
        else:
            async with manager:
                await work()
    except Error:
        recover()
    finally:
        finish()
    completed()
