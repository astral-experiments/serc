def f(value, obj, items):
    if value is None:
        if obj.flag:
            values = tuple(item.value for item in items)
            value = obj.make(values)
        else:
            values = tuple(item.value for item in items)
            value = obj.make(values)
    else:
        assert value == obj.other
    value = obj.finish(value)
    return obj.wrap(value)
