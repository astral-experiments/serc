class C:
    def build(self, values):
        __x = 1
        function = lambda __x: _C__x
        closures = [(lambda: _C__x) for __x in values]
        generator = (__x for _C__x in values)
        return function, closures, generator
