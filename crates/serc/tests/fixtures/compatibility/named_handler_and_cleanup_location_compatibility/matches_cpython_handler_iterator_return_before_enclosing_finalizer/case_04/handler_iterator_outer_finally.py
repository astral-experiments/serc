async def f(items, flag, value):
    try:
        work()
    except Error as error:
        for item in items:
            try:
                return value
            except InnerError:
                recover()
    finally:
        cleanup()
    return value
