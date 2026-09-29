async def f(items, flag, value):
    try:
        work()
    except:
        for item in items:
            try:
                return
            except InnerError:
                recover()
    finally:
        cleanup()
    return value
