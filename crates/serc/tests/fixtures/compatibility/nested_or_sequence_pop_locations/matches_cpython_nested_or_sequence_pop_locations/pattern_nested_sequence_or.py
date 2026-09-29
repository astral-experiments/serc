def f(subject):
    match subject:
        case Box(
            args=[int(value) | str(value), *_],
        ):
            return value
    return None
