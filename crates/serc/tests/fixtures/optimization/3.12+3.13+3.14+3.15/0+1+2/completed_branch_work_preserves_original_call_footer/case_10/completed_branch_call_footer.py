def f(a, b, c, obj, items):
    with obj:
        if a:
            assert a
        if b:
            if c:
                obj.work()
            obj.finish()
