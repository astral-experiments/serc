def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([a for a in values], [b for b in values], a) for middle in values]], b) for root in values]
        return result
    return function
