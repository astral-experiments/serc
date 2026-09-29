def f(a, b, c, obj, items):
    if a:
        assert a
    if b:
        obj.work([item for item in items])
        obj.finish()
