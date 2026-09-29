def f(subject):
    match subject:
        case C(
            a=a,
            b=b,
            c=c
        ):
            return make(a, b, c)
        case _:
            return fallback(subject)
