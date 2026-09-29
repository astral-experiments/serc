def f(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
            work()
        work()
    except Exception:
        recover()
