def function(manager, value, fallback):
    with manager:
        match value:
            case Kind.FIRST:
                return handle(value)
            case _:
                fallback(value)
