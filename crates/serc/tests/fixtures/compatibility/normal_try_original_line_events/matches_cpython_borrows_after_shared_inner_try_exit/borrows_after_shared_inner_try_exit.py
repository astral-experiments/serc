def f(outer, inner, value, work):
    try:
        value = work(value)
        if outer:
            try:
                outer = work(outer)
                if inner:
                    if value is None:
                        value = []
                    value.append(work(inner))
            except Exception:
                pass
    except Exception:
        return None
    if len(value) == 0:
        return None
    work(value)
    return [work(value), value, value]
