def f(value):
    try:
        match value:
            case C(
                first=a,
                second=B() | D() as b):
                return a, b
    except Exception:
        return value
