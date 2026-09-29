def f(value, result):
    match value:
        case int():
            match result:
                case str():
                    return result
                case _:
                    pass
        case _:
            pass
    return result
