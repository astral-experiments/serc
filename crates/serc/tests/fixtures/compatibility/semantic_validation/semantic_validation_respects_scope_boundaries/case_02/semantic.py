def f():
    yield 1
    def g(x=(yield 2)): pass
