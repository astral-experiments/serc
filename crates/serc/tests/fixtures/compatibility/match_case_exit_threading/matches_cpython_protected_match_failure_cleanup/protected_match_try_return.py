def select(value, first, second, fallback):
    try:
        match value:
            case "first":
                return first()
            case "second":
                return second()
    except ValueError:
        return fallback()
    raise ValueError("unmatched")
