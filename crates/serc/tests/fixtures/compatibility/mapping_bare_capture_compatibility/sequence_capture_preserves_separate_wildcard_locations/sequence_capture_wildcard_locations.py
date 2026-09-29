def f(subject):
    match subject:
        case [
            value,
            _,
            _,
        ]:
            return value
        case _:
            return None
