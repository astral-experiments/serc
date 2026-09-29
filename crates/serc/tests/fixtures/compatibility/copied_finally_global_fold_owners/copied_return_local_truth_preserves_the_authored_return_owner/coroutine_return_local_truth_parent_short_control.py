async def f(flag, ready):
    try:
        if ready:
            report(first())
            return
        await work()
    except Exception as error:
        report(error)
    finally:
        if flag:
            await cleanup()
