def f(work, pair):
    try:
        work()
    except ValueError as error:
        match pair():
            case (False, True):
                return error
            case (True, False):
                return None
    return False
