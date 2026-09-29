def f(manager):
    try:
        with manager:
            pass
            try:
                return work()
            finally:
                cleanup()
    except Exception:
        recover()
