def enclosing(probe):
    def function(values):
        result = [[lambda: probe for probe in values] for probe.attr in values]
        return result
    return function
