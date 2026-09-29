def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([x for x in lookup(values)], [y for y in other(values)], [lookup for lookup in values], [other for other in values]) for item in values]
        return result
    return function
