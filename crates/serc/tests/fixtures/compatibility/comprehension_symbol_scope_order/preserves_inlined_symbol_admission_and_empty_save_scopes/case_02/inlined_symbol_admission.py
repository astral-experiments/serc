def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [((lambda value=probe: value), [(lambda: probe) for probe in values]) for outer in values]
        return result
    return function
