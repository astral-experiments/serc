def f(a, b, c, obj, items):
    if a:
        assert a
    if b:
        for item in items:
            if item:
                continue
            obj.work(item)
        obj.finish()
