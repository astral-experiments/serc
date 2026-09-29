def f(work, values, get):
    try:
        work()
    finally:
        while values:
            get(values.pop())
