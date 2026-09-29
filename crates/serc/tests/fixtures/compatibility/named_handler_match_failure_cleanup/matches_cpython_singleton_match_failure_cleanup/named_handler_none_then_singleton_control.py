def f(work, pair):
    try:
        work()
    except ValueError as error:
        match pair():
            case (None, True):
                return error
            case (True, False):
                return None
    return False
