def f(manager, subject):
    with manager:
        match subject:
            case (False, error):
                return error
            case (True, result, name):
                pass
            case _:
                raise ValueError()
        return result
