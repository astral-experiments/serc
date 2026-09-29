def target(outer, direct, value, check, transform):
    try:
        if outer:
            if direct:
                return transform(value)
            else:
                value = transform(value)
                if value and check(value):
                    return value
        elif check(outer):
            value = transform(value)
            if value:
                return value
    except Exception:
        return ""
    return ""
