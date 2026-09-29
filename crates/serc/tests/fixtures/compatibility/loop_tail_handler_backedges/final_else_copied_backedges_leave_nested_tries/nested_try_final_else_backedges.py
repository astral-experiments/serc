def f(items, debug, flag):
    for item in items:
        try:
            if not debug:
                consume(item)
            else:
                try:
                    work(item)
                    if flag:
                        consume(item)
                except ValueError as error:
                    report(error)
                    pass
        except RuntimeError as error:
            report(error)
            pass
