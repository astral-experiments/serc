def f(value):
    match value:
        case C(
            first=a,
            second=B() | D() as b):
            return a, b
