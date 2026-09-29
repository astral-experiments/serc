def f(flag, other, manager):
    try:
        with manager:
            if flag or other:
                assert flag
            else:
                assert not flag
            work()
        work()
    except Exception:
        recover()
