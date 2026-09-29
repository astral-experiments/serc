class Container:
    def method(self, __value):
        self.__field = __value
        result = self.__field
        del self.__field
        return result, super().__repr__()


def leaves(values, source):
    first_small = 7
    later_small = 7
    huge = 123456789012345678901234567890
    negative_zero = -0.0
    complex_value = 2j
    text = "implicit " "concatenation"
    data = b"adjacent " b"bytes"
    observed = ((captured := item) for item in values)
    wrapped = [item for item in values].__class__
    named = (captured := source).__class__
    return (
        __debug__, None, True, False, ..., first_small, later_small, huge,
        negative_zero, complex_value, text, data, observed, wrapped, named,
    )


def yielding(values):
    return (yield values).__class__


async def nested(values):
    return [(__debug__, item, b"x", "y") async for item in values]
