def collect(manager, ready, work, pending, buffered):
    with manager():
        while True:
            yield None
            if ready():
                try:
                    work()
                except LookupError:
                    if pending() == 0:
                        break
            else:
                work()
