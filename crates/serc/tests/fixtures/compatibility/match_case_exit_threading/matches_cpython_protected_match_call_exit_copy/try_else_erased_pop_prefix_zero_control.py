def f(value, enabled):
    try:
        result = value
    except Error:
        return None
    else:
        if enabled:
            record(result)
    return result
