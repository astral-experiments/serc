def f(subject, convert):
    match subject:
        case {"value": captured}:
            try:
                return convert(captured)
            except ValueError:
                return None
    return None
