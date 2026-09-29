def guarded_final(subject, use):
    match subject:
        case {"skip": True}:
            return []
        case {"key": value} if use(value):
            return value
        case _:
            return []

def guarded_single(subject, use):
    match subject:
        case {"key": value} if use(value):
            return value
        case _:
            return []
