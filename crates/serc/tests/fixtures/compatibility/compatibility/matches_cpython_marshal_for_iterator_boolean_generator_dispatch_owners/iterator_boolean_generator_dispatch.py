def direct_any(groups):
    for values in groups:
        if any(not value for value in values):
            continue
        yield values


def direct_all(groups):
    for values in groups:
        if all(value for value in values):
            break
        yield values


def disjunction(groups):
    for values in groups:
        if not values or any(not value for value in values):
            continue
        yield values


def conjunction(groups, first, second):
    for values in groups:
        if first and second and all(value for value in values):
            continue
        yield values


def prior_yield(groups):
    for values in groups:
        yield None
        if not values or any(not value for value in values):
            continue
        yield values


def prior_assignment(groups):
    for group in groups:
        values = [value for value in group]
        if not values or any(not value for value in values):
            return
        yield values


def first_yield_else(groups, flag):
    for values in groups:
        if flag or any(not value for value in values):
            yield values
        else:
            consume(values)


def first_yield_elif(groups, flag, other):
    for values in groups:
        if flag or all(value for value in values):
            yield values
        elif other:
            consume(values)


def boolean_value(groups, flag, sink):
    for values in groups:
        result = flag or sink((any(not value for value in values), values))
        yield result


def plain(groups, flag):
    for values in groups:
        if flag or any(not value for value in values):
            continue
        consume(values)


async def coroutine(groups, first, second):
    for values in groups:
        if first and second and any(not value for value in values):
            continue
        await consume(values)


async def coroutine_after_await(groups, flag):
    for values in groups:
        await prepare(values)
        if flag or all(value for value in values):
            continue
        await consume(values)


async def coroutine_first_await(groups, flag):
    for values in groups:
        if flag or any(not value for value in values):
            await consume(values)
        else:
            prepare(values)


async def asynchronous_generator(groups, flag):
    for values in groups:
        if flag or all(value for value in values):
            continue
        yield values
