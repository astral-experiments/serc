def f(values, flag):
    while flag:
        try:
            return values[1]
        finally:
            return values[0]
