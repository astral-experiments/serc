def select(value, first, second, fallback):
    try:
        if value == "first":
            first()
        elif value == "second":
            second()
    except ValueError:
        fallback()
    return value
