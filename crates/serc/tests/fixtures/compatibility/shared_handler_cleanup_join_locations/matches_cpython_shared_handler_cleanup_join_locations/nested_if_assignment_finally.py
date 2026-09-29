async def f(work, outer, middle, inner, after):
    try:
        await work()
    except Exception as error:
        value = error
        if outer:
            if middle:
                if inner:
                    work()
                else:
                    work(error)
                work()
            value = False
    finally:
        after()
