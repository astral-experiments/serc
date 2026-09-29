def f(value, items, obj, flag):
    if value is None:
        return value
    else:
        if items is not None:
            for item in items:
                if item in value:
                    return value
            raise ValueError(value)
        pass
