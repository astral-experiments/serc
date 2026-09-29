def f(stop, wait, record, value):
    try:
        while True:
            try:
                wait(stop)
                break
            except TimeoutError:
                value = stop.current()
                if value is not None:
                    record(value)
    except RuntimeError:
        pass
