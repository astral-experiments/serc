def f(value):
    match value:
        case C(
            first=a,
            second=b,
            third=c,
        ):
            return a, b, c
