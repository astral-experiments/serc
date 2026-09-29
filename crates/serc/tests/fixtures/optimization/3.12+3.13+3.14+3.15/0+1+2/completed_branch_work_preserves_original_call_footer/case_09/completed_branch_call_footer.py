def f(a, b, c, obj, items):
    if a:
        assert a
    if b:
        with obj:
            obj.work()
        obj.finish()
    obj.after()
