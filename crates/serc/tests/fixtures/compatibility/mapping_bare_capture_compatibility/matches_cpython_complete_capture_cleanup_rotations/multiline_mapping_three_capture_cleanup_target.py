def f(value):
    match value:
        case {
            "first": first,
            "second": second,
            "flag": False,
            "last": last,
        } if isinstance(first, str):
            return first, second, last
    return None
