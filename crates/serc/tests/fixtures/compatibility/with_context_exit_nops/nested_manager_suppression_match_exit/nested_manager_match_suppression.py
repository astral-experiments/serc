
def make_runner(outer, inner, kind):
    with outer:
        match kind:
            case "primary":
                with inner:
                    def run(value):
                        return consume(value)
                    return run
            case _:
                raise ValueError(kind)
