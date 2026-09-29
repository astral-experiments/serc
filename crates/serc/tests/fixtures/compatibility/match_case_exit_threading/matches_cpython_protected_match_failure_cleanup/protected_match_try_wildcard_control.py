def select(value, first, second, fallback):
    try:
        match value:
            case "first":
                return first()
            case "second":
                return second()
            case _:
                return fallback()
    except ValueError:
        return fallback()
    raise ValueError("unmatched")
