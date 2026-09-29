def f(value, route, kind, mode, allowed, replacement, touch):
    if route:
        if kind == 1:
            if mode:
                if replacement is not None:
                    value = replacement
            else:
                touch()
                if allowed:
                    if replacement is not None:
                        value = replacement
        elif kind == 2:
            touch()
        else:
            touch()
    return value
