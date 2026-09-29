def f(manager, flag, items):
    try:
        with manager:
            pass
            raise ValueError()
    except Exception:
        recover()
