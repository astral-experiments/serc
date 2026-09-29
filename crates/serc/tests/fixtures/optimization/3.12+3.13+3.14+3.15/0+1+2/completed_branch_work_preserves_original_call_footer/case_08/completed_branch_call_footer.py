def f(a, b, c, obj, items):
    if a:
        pass
    if b:
        with obj:
            obj.work()
        obj.finish()
