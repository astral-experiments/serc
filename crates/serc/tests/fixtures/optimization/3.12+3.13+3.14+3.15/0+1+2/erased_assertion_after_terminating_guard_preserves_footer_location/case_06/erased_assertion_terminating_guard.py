def f(value, items, obj, flag):
    if value is None:
        return value
    else:
        if items is not None:
            try:
                obj.work()
            except ValueError:
                obj.recover()
            raise ValueError(value)
        assert False
