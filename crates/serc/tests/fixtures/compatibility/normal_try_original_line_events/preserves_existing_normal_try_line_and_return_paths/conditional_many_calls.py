def f(flag, work, recover):
    try:
        if flag():
            work()
            work()
            try:
                work()
            except ValueError:
                recover()
    except Exception:
        recover()
    return work()
