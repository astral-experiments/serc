def f(a, b, c, obj, items):
    if a:
        assert a
    if b:
        with obj:
            with obj.other:
                for item in items:
                    if item:
                        continue
                    obj.work(item)
        obj.work([item for item in items])
        obj.finish()
