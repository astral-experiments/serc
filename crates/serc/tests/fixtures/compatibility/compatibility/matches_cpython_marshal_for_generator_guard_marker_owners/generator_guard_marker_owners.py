def after_resume(values):
    yield before()
    result = tuple(value for value in values)
    yield result


def while_fresh(values):
    yield before()
    while True:
        result = tuple(value for value in values)
        yield result


def after_dynamic(flag, values):
    yield before()
    if flag:
        prepare()
        result = tuple(value for value in values)
        yield result


def iterator_owned(groups):
    yield before()
    for values in groups:
        result = tuple(value for value in values)
        yield result


def consecutive_fresh(values):
    yield before()
    while True:
        first = tuple(value for value in values)
        second = tuple(value for value in values)
        yield first, second
