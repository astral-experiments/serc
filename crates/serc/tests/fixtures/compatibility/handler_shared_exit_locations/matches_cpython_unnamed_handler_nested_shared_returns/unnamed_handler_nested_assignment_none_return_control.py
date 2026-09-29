def update_value(value, first, second):
    try:
        check(value)
    except ValueError:
        if first:
            if second:
                value.primary = 1
            else:
                value.secondary = 2
        else:
            value.fallback = 3
    return None
