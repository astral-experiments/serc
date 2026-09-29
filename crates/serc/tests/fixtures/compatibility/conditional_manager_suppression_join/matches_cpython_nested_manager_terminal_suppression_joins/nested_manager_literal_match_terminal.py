async def run(self, outer, inner, read):
    with outer():
        with inner() as value:
            match self.kind:
                case "a":
                    return read(value, "a")
                case "b":
                    return read(value, "b")
                case "c":
                    return read(value, "c")
                case _:
                    raise RuntimeError("unknown kind")
