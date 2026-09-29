async def f(flag, other):
    try:
        await work()
    except Error as error:
        await report(error)
        if not flag or other:
            await recover(error)
