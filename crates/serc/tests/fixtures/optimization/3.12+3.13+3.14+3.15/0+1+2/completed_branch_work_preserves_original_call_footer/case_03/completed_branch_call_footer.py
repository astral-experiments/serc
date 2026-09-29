def f(a, b, c, obj, items):
    if a:
        assert a
    if b:
        with obj:
            with obj.other:
                obj.work()
        obj.finish()
