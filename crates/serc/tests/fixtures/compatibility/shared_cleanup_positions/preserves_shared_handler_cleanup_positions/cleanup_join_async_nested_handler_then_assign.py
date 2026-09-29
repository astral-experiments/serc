async def f(flag, values, manager, other):
    try:
        work()
    except Error as error:
        report()
        if flag and other:
            try:
                await primary()
            except Other:
                await fallback()
            result = None
    finish()
