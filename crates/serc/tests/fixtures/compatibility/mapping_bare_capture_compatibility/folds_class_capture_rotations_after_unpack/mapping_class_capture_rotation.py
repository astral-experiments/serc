def single(subject):
    match subject:
        case {"value": str(value)}:
            return value
    return None


def pair(subject):
    match subject:
        case {"left": str(left), "right": int(right)}:
            return left, right
    return None


def pair_as(subject):
    match subject:
        case {"left": str() as left, "right": str() as right}:
            return left, right
    return None
