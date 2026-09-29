def f(manager, flag, items):
    try:
        try:
            with manager:
                pass
                raise ValueError()
        finally:
            pass
    except Exception:
        recover()
    yield 1
