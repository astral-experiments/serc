def retry(manager, running, accepted, stop):
    with manager:
        while running:
            try:
                action()
            except Error as error:
                record(error)
                if accepted:
                    refresh()
                    if stop:
                        record(error)
                        break
                else:
                    raise error
    finish()
