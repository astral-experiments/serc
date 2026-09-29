def f(flag, stop, first, second):
    while flag:
        if stop:
            break
        try:
            assert first or second
        except Exception:
            pass
