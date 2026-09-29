def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [([probe for probe in values], [(lambda: probe) for x in values]) for outer in values]
        return result
    return function
