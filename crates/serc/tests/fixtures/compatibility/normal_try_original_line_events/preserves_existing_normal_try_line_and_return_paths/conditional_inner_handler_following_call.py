def f(flag, work, recover):
    try:
        if flag:
            try:
                work()
            except ValueError:
                recover()
    except Exception:
        recover()
    work()
    return flag
