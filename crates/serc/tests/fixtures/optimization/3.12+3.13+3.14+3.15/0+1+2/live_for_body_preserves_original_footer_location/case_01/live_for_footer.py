def f(obj, items):
    if obj.ready:
        assert obj.ready
    if obj.active and obj.target is not None:
        for item in items:
            item.action()
