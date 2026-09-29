def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [[[(lambda: probe) for probe in probe(values)] for probe in values] for outer in values]
        return result
    return function
