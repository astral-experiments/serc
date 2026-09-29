def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([x for x in lookup(values)], [(lambda: lookup) for lookup in values]) for item in values]
        return result
    return function
