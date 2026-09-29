def f(xs):
    result = [x for x in xs if any(test(y) for y in xs)]
    assert result
