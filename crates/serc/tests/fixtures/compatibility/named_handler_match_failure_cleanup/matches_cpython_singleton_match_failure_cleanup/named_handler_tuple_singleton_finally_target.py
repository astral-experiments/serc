def f(run, work, pair, report, finish):
    try:
        while run():
            try:
                work()
            except Exception as error:
                match pair():
                    case (False, True):
                        report(error)
                        return
                    case (False, False):
                        report(error)
                    case (True, _):
                        pass
    finally:
        finish()
