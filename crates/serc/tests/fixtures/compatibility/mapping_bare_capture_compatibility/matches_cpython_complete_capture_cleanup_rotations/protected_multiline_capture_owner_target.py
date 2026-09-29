def f(value, finish):
    try:
        match value:
            case {
                "first": first,
                "second": second,
                "flag": False,
                "last": last,
            } if first is not None:
                return first, second, last
        return None
    finally:
        finish()
