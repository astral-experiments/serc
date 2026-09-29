def f(work, get):
    try:
        work()
    finally:
        while True:
            try:
                get()
            except LookupError:
                break
