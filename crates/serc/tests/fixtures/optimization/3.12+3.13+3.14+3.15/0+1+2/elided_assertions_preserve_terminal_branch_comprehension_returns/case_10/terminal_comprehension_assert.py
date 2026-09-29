def f(flag, xs, manager):
    with manager:
        if flag:
            result = [x for x in xs]
            assert result
