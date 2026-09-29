def enclosing(lookup):
    def function(values):
        result = {0: [lookup(v) for v in values], tuple([lookup for lookup in values]): 0}
        return result
    return function
