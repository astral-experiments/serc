def enclosing(probe):
    def function(values):
        nonlocal probe
        prelude = [probe(value) for value in values]
        result = [([(lambda: probe) for probe in values], (probe := item)) for item in values]
        return result
    return function
