GLOBAL = 10
groups = ((1, 2),)

def decorate(value):
    return lambda function: function

class Example:
    source = (1, 2, 3)
    callbacks = (str,)
    values = [GLOBAL + value for value in source]
    labels = {str(value) for value in source if bool(value)}
    mapping = {value: tuple((GLOBAL, value)) for value in source}
    nested = [[str(inner) for inner in range(outer)] for outer in source]
    called = [callback(value) for callback in callbacks for value in source]
    shadowed = [GLOBAL for GLOBAL in source]

def enclosing(captured):
    class Nested:
        values = [GLOBAL + captured + value for value in range(3)]

        def method(self):
            return __class__, captured

    return Nested

class Decorated:
    @decorate([item for item in group] for group in groups)
    def method(self):
        return None
