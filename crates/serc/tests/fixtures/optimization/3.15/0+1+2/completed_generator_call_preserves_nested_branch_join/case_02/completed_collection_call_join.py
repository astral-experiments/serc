def f(value, obj, items):
    if value is None:
        if obj.flag:
            values = set(item.value for item in items)
            value = obj.make(values)
        else:
            obj.check()
            value = obj.build()
    else:
        assert value == obj.other
    value = obj.finish(value)
    return obj.wrap(value)
