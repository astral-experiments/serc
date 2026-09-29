def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [[(lambda: probe) for dummy in values for probe in probe(values)] for outer in values]
        return result
    return function
