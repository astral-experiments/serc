def f(values):
    for value in values:
        for other in values:
            try:
                return value
            finally:
                return other
