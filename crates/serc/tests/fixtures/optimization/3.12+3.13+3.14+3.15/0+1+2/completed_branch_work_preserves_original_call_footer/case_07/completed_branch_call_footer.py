def f(a, b, c, obj, items):
    if b:
        with obj:
            obj.work()
        obj.finish()
