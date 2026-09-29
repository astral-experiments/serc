def f(value):
    match value:
        case (str(a), str(b), c):
            return a, b, c
