def run(retry, marked):
    while True:
        try:
            prepare()
            try:
                action()
            except Fatal as error:
                report(error)
                if not marked:
                    record()
                    marked = True
                stop()
            except Error as error:
                delay = trim()
                report(error)
                stop()
            else:
                report()
                retry = 0
                marked = False
        except Error as error:
            report(error)
