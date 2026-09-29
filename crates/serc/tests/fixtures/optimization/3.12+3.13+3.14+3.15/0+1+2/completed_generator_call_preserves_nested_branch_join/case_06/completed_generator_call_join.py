def f(value, obj, items):
    if value is None:
        if obj.flag:
            value = obj.make()
        else:
            obj.check()
            value = obj.build()
    else:
        assert value == obj.other
    value = obj.finish(value)
    return obj.wrap(value)
