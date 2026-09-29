def enclosing(lookup):
    def function(values, holder):
        holder[tuple([lookup for lookup in values])] += [lookup(v) for v in values]
        return None
    return function
