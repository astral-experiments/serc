def f(work, pair, finish):
    try:
        try:
            work()
        except Exception as error:
            match pair():
                case (1, "ready"):
                    return error
                case (2, "stopped"):
                    return None
    finally:
        finish()
