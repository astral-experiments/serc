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
                if retry:
                    retry += 1
                    report(error)
                else:
                    report(error)
                    if not marked:
                        record()
                        marked = True
                    stop()
            else:
                report()
                retry = 0
                marked = False
        except Error as error:
            report(error)
