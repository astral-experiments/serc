def f(value, items, obj, flag):
    if value is None:
        return value
    else:
        if items is not None:
            obj.work()
            raise ValueError(value)
        assert False
