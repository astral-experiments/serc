def enclosing(lookup):
    def function(values, holder):
        result = [0 for item in values for holder[tuple([lookup for lookup in values])] in [lookup(v) for v in values]]
        return result
    return function
