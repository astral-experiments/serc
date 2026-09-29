def function(
    sequence: [lambda: item for item in (1,)],
    unique: {lambda: item for item in (2,)},
) -> {key: lambda: key for key in (3,)}:
    pass

def outer(values):
    def inner(argument: [lambda: item for item in values]):
        pass
    return inner
