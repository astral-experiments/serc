def direct(value):
    try:
        assert value
    except ValueError:
        recover()
    return value


def message(value):
    try:
        assert value, "bad"
    except ValueError:
        recover()
    return value


def with_else(value):
    try:
        assert value
    except ValueError:
        recover()
    else:
        consume(value)
    return value


def multiple_handlers(value):
    try:
        assert value
    except ValueError:
        recover()
    except TypeError:
        recover()
    return value


def nonfinal(value):
    try:
        assert value
        consume(value)
    except ValueError:
        recover()
    return value


def constant_true(value):
    try:
        assert True
    except ValueError:
        recover()
    return value


def constant_false(value):
    try:
        assert False
    except AssertionError:
        recover()
    return value


def boolean_and(left, right):
    try:
        assert left and right
    except ValueError:
        recover()
    return right


def boolean_or(left, right):
    try:
        assert left or right
    except ValueError:
        recover()
    return right


def conditional(value, alternate, flag):
    try:
        assert value if flag else alternate
    except ValueError:
        recover()
    return value


def protected(manager, value):
    with manager:
        try:
            assert value
        except ValueError:
            recover()
    return value


def protected_loop(manager, values):
    with manager:
        for value in values:
            try:
                assert value
            except ValueError:
                recover()


def terminal(value):
    try:
        assert value
    except ValueError:
        recover()


def generator(value):
    try:
        assert value
    except ValueError:
        recover()
    yield value


async def coroutine(value):
    try:
        assert value
    except ValueError:
        recover()
    return value
