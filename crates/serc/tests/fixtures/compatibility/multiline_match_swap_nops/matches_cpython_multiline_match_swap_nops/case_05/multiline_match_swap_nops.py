def f(value):
    try:
        match value:
            case C(
                first=A(x=a),
                second=B(item=[_]) as b,
            ):
                return a, b
    except Exception:
        return value
