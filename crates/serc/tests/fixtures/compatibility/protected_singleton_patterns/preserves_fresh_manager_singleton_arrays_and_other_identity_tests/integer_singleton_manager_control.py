def f(manager, subject):
    with manager:
        match subject:
            case (0, error):
                return error
            case (1, result, name):
                pass
            case _:
                raise ValueError()
        return result
