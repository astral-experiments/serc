def enclosing(probe):
    def function(values, holder):
        ordinary = probe
        result = [None for item in values for holder[[probe(x) for x in values]] in [(lambda: probe) for probe in values]]
        return result
    return function
