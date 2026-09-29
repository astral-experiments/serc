def enclosing(probe):
    def function(values):
        result = [[lambda: probe for probe in values] for probe[0] in values]
        return result
    return function
