def select(value, manager, first, second):
    with manager():
        match value:
            case "first":
                return first()
            case "second":
                return second()
    raise ValueError("unmatched")
