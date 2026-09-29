def f(manager):
    try:
        with manager:
            pass
            try:
                yield work()
            finally:
                cleanup()
    except Exception:
        recover()
