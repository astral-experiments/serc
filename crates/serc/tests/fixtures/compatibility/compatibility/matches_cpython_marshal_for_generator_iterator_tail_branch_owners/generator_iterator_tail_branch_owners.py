def sync_assigned(values, first, second):
    for value in values:
        prepared = first(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


def sync_nested(values, first, second):
    pending = [values]
    while pending:
        for value in pending.pop():
            if first(value):
                pending.append(value)
            else:
                yield second(value)


def sync_prior_yield(values, first, second):
    for value in values:
        yield first(value)
        prepared = first(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


async def async_direct(values, first, second):
    async for value in values:
        if first(value):
            yield first(value)
        else:
            yield second(value)


async def async_assigned(values, first, second):
    async for value in values:
        prepared = first(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


async def async_protected(values, first, second, cleanup):
    try:
        async for value in values:
            prepared = first(value)
            if prepared:
                yield prepared
            else:
                yield second(value)
    finally:
        cleanup()


async def coroutine_sync_iterator(values, first, second):
    for value in values:
        prepared = first(value)
        if prepared:
            first(value)
        else:
            second(value)


async def coroutine_after_await(values, ready, first, second):
    await ready()
    for value in values:
        prepared = first(value)
        if prepared:
            first(value)
        else:
            second(value)


async def coroutine_async_iterator(values, first, second):
    async for value in values:
        prepared = first(value)
        if prepared:
            first(value)
        else:
            second(value)


def prior_condition(values, first, second, early):
    for value in values:
        if early(value):
            first(value)
        prepared = first(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


def prior_continue(values, first, second, early):
    for value in values:
        if early(value):
            continue
        prepared = first(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


async def prior_await(values, ready, first, second):
    for value in values:
        prepared = await ready(value)
        if prepared:
            first(value)
        else:
            second(value)


async def async_prior_condition(values, first, second, early):
    async for value in values:
        if early(value):
            first(value)
        prepared = first(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


async def async_prior_await(values, ready, first, second):
    async for value in values:
        prepared = await ready(value)
        if prepared:
            yield prepared
        else:
            yield second(value)


def prior_nested_iterator(values, groups, first, second, flag):
    for value in values:
        for other in groups:
            first(other)
        prepared = first(value)
        if flag:
            break
        else:
            yield second(value)


def prior_try_finally(values, first, second, cleanup, flag):
    try:
        for value in values:
            try:
                prepared = first(value)
            finally:
                cleanup()
            if flag:
                continue
            else:
                yield second(value)
    except RuntimeError:
        cleanup()
