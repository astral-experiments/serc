def f(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
            pass
        work()
    except Exception:
        recover()
