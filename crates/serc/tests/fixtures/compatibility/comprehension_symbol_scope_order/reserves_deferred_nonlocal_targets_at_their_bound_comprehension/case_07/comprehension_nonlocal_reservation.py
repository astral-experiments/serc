def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([(lambda: lookup) for lookup in values], [(lambda: other) for other in values], [x for x in lookup(values)], [y for y in other(values)]) for item in values]
        return result
    return function
