def select(value, first, second, fallback):
    try:
        match value:
            case "first":
                first()
            case "second":
                second()
    except ValueError:
        fallback()
    return value
