def function(manager, value, prepare, fallback):
    with manager:
        match value:
            case Kind.FIRST:
                return handle(value)
            case _:
                prepare(value)
                fallback(value)
