def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([(lambda: lookup) for lookup in values], [x for x in lookup(values)]) for item in values]
        return result
    return function
