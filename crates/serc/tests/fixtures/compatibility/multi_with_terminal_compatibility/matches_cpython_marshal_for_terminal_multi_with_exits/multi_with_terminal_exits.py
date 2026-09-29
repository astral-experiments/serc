def nested_raise(outer, first, second, error):
    with outer:
        with first, second:
            raise error
    verify(error)

def nested_multiline_raise(outer, first, second, error):
    with outer:
        with (
            first,
            second,
        ):
            raise error
    verify(error)

def nested_call_raise(outer, first, second, error):
    with outer():
        with first(), second():
            raise error
    verify(error)

def nested_asserted_raise(outer, first, second, error):
    with outer as value:
        with first, second:
            raise error
    assert value is error

def same_line_return(first, second):
    with first, second:
        return value()

def multiline_return(first, second):
    with (
        first,
        second,
    ):
        return value()

def multiline_three_return(first, second, third):
    with (
        first,
        second,
        third,
    ):
        return value()

def multiline_raise(first, second):
    with (
        first,
        second,
    ):
        raise Error

def multiline_nested_return(outer, first, second):
    with outer:
        prepare()
        with (
            first,
            second,
        ):
            return value()

async def coroutine_multiline_return(first, second):
    with (
        first,
        second,
    ):
        return await value()

def multiline_call_contexts_return(first, second):
    with (
        first(),
        second(),
    ):
        return value()

def multiline_call_contexts_raise(first, second):
    with (
        first(),
        second(),
    ):
        raise Error

def multiline_as_return(first, second):
    with (
        first() as left,
        second() as right,
    ):
        return value(left, right)
