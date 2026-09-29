def f(a, b, c):
    try:
        g()
    finally:
        assert a is b and not a.value
        g()
