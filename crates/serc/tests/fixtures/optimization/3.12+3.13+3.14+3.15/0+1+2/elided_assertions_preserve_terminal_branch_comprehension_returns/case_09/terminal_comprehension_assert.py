def f(flag, xs):
    try:
        if flag:
            result = [x for x in xs]
            assert result
    finally:
        cleanup()
