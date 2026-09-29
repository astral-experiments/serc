def f(flag, xs):
    if flag:
        result = {x: x for x in xs}
        assert result
