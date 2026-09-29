def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [[(lambda: probe) for probe in (probe(values) if flag else values)] for outer in values]
        return result
    return function
