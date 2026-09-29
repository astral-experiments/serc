def f(manager, x, y):
    if x:
        try:
            with manager:
                pass
                work()
        except Exception:
            recover()
        if y:
            pass
        assert y
