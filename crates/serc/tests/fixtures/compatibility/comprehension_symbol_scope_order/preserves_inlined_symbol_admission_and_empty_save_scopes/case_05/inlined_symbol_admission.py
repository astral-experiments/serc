class Example:
    def function(self, values):
        ordinary = __class__
        result = [(super(), [lambda: __class__ for __class__ in values]) for item in values]
        return result
