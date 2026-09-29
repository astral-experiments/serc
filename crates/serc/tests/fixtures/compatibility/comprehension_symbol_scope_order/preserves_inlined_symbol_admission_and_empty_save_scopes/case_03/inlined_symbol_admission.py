def enclosing(probe):
    def function(values):
        nonlocal probe
        prelude = [probe(value) for value in values]
        result = [((probe := item), [(lambda: probe) for probe in values]) for item in values]
        return result
    return function
