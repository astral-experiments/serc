def f(outer, inner, value):
    with outer:
        with inner:
            return value
