def f(subject):
    match subject:
        case C(
            a=a, b=b, c=c
        ):
            result = make(a, b, c)
        case _:
            result = fallback(subject)
    return result
