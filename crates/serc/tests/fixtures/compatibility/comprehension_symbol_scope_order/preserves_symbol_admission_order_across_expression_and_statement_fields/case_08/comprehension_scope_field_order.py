def enclosing(lookup):
    def function(values, holder):
        for holder[tuple([lookup for lookup in values])] in [lookup(v) for v in values]:
            pass
        return None
    return function
