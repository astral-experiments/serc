def select_value(value, first, second):
    result = value
    try:
        check(value)
    except ValueError:
        if first:
            if second:
                result = value.primary
            else:
                result = value.secondary
        else:
            result = value.fallback
    return result
