def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [([(lambda: probe) for probe in values], probe) for outer in values]
        return result
    return function
