def f(obj, items):
    if obj.ready:
        assert obj.ready
    if obj.active:
        for item in items:
            item.action()
