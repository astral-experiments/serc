def enclosing(probe):
    def function(values):
        ordinary = probe
        x = None
        result = [{0: [(lambda: probe) for probe in values], tuple([probe(x) for x in values]): 0} for item in values]
        return result
    return function
