def f(value, result, work):
    match value:
        case "a":
            work()
            pass
        case "b":
            work()
        case _:
            raise ValueError(value)
    return result
