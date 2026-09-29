def f(xs):
    result = {
        x: sum(test(x, y) for y in xs)
        for x in xs
    }
    assert result
