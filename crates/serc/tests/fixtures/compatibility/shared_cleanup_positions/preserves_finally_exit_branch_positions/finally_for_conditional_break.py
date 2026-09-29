def f(work, values, get, condition):
    try:
        work()
    finally:
        for value in values:
            if condition(value):
                break
            get(value)
