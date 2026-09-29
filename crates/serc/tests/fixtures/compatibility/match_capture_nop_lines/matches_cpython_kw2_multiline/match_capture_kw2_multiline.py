def f(subject):
    match subject:
        case C(
            a=a, b=b
        ):
            return make(a, b)
        case _:
            return fallback(subject)
