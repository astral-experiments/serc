def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in {0: [([(lambda: a) for a in values], [(lambda: b) for b in values], a) for middle in values]}[0]], b) for root in values]
        return result
    return function
