def f(items, flag, value):
    try:
        work()
    except Error:
        for item in items:
            try:
                if flag:
                    return None
            except InnerError:
                recover()
    finally:
        if flag:
            cleanup()
    return value
