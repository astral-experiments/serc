def f(items, flag, value):
    try:
        work()
    except:
        for item in items:
            try:
                return value
            except InnerError:
                recover()
    finally:
        if flag:
            cleanup()
    return value
