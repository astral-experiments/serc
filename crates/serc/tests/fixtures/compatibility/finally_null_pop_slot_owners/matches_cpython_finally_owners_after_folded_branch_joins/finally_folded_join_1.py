async def f(value, flag, other, task, manager):
    await prepare()
    try:
        result = 1 if flag else 2
        work()
        work()
        work()
        if False:
            task(value)
    finally:
        if value:
            await value.close()
    after()
