def function_defaults():
    def inner(value=(yield 1), *, other=(yield from (2, 3))):
        return value, other

    return inner


def function_decorator():
    @(yield 1)
    def inner():
        pass


def async_function_default():
    async def inner(value=(yield 1)):
        return value


def generic_function_defaults():
    def inner[T](value=(yield 1), /, *, other=(yield from (2, 3))):
        pass


def lambda_defaults():
    return lambda value=(yield 1), *, other=(yield from (2, 3)): (value, other)


async def async_generator_default():
    def inner(value=(yield 1)):
        return value


def discarded_header():
    if False:
        def inner(value=(yield 1)):
            pass
    return 2


nested_lambda = lambda: (lambda value=(yield 1): value)
nested_lambda_keyword = lambda: (lambda *, value=(yield from (1, 2)): value)
nested_lambda_recursive = lambda: (lambda value=(lambda default=(yield 1): default): value)


def independent_bodies():
    def inner():
        yield 1

    class Container:
        def method(self):
            yield 2

    return inner, Container, lambda: (yield 3)


def independent_annotation():
    def inner(value: (lambda: (yield 1))):
        pass

    return inner


independent_lambda = lambda: (lambda: (yield 1))
