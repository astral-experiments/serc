def f(outer, inner, value, ignore):
    with outer:
        try:
            with inner:
                return value
        except:
            if ignore:
                return None
            else:
                raise
