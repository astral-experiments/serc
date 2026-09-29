
def make_runner(outer, inner, kind):
    with outer:
        if kind == "primary":
            with inner:
                def run(value):
                    return consume(value)
                return run
        else:
            raise ValueError(kind)
