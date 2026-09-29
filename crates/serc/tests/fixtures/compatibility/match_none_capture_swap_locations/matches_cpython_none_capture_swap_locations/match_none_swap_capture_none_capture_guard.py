def f(value):
    match value:
        case (t, None, u) if u:
            return t, u
