def f(flag, work):
    try:
        if flag:
            work()
    except Exception:
        work()
    work()
    return flag
