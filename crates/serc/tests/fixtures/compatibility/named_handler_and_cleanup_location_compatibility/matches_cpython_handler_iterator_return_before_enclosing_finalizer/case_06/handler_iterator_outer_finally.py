async def f(items, flag, value):
    try:
        work()
    except Error:
        for item in items:
            try:
                if flag:
                    return value
            except InnerError:
                recover()
    finally:
        if flag:
            cleanup()
    return value
