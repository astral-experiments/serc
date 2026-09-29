def f(value, enabled, kind, size, manager):
    with manager:
        if enabled:
            if kind == 1:
                assert len(value) == size
            elif kind == 2:
                assert len(value) == 2
            else:
                raise ValueError()
    return value
