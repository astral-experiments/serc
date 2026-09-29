def f(subject, allowed):
    match subject:
        case Box(
            args=[int(value) | str(value), *_],
        ) if value in allowed:
            return value
    return None
