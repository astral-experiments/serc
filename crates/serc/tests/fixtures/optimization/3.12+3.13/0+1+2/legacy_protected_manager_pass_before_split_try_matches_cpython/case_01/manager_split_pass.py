def f(manager):
    try:
        with manager:
            pass
            try:
                work()
            finally:
                cleanup()
    except Exception:
        recover()
