async def f(manager, copy, size, finish):
    with manager:
        try:
            copy()
        except Exception as error:
            report("failed")
            for i in range(5):
                try:
                    copy()
                    break
                except Exception as error:
                    report(i, error)
        value = size()
    return finish(value)
