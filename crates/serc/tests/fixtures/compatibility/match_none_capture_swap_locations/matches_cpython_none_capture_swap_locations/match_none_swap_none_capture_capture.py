def f(value):
    match value:
        case (None, t, u):
            return t, u
