def f(enabled, condition, stop, work, recover):
    try:
        if enabled:
            while condition():
                if stop():
                    break
                work()
    except Exception:
        recover()
    work()
    return enabled
