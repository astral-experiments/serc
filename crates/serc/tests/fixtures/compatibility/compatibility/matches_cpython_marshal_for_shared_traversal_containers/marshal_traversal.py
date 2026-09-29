def outer():
    def nested(value):
        return (
            value in {("alpha", 1), ("beta", 2)},
            value[257:256:2],
            ((1, 2), (1, 2)),
        )

    return nested
