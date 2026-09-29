def enclosing(lookup):
    def function(values, holder):
        result = {tuple([lookup(v) for v in values]): [lookup for lookup in values] for item in values}
        return result
    return function
