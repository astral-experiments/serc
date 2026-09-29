def enclosing(lookup):
    def function(values):
        @[lookup(v) for v in values]
        def child(arg=[lookup for lookup in values]):
            return arg
        return child
    return function
