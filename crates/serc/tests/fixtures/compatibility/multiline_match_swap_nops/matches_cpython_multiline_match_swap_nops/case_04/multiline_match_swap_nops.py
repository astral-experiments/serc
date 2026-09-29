def f(value):
    match value:
        case A(a, b, c) | B(
            A(a, b, c)
        ):
            return a, b, c
