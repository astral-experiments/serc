def f(outer, inner, ignore):
    with outer:
        prepare()
        try:
            with inner as stream:
                value = read(stream)
                return value
        except:
            if ignore:
                return None
            else:
                raise
