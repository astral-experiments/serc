def outer(value):
    class C:
        value = 10
        def f(x: [value + i for i in (1,)]): pass
    return C
