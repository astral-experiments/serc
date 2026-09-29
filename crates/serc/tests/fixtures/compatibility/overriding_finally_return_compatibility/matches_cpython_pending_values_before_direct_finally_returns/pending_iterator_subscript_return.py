def f(values):
    for value in values:
        try:
            return value
        finally:
            return values[0]
