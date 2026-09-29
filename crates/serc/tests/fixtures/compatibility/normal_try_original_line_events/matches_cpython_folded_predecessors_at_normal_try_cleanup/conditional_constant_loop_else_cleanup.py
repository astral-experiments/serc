def f(enabled, stop, work, recover):
    try:
        if enabled:
            while True:
                if stop():
                    break
                work()
            else:
                work()
    except Exception:
        recover()
    work()
    return enabled
