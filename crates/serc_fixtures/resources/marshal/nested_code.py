def outer(seed):
    captured = seed

    def first(value):
        return captured + value

    def second(value):
        def third(extra):
            return captured + value + extra

        return third

    class Nested:
        value = captured

        def method(self, value):
            return first(value)

    lambdas = (lambda value: value, lambda value: captured + value)
    comprehensions = [first(value) for value in range(10)]
    generator = (second(value) for value in range(10))
    return first, second, Nested, lambdas, comprehensions, generator
