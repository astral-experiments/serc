def f(flag, work, recover):
    try:
        if flag:
            try:
                work()
            finally:
                recover()
    except Exception:
        recover()
    work()
    return flag
