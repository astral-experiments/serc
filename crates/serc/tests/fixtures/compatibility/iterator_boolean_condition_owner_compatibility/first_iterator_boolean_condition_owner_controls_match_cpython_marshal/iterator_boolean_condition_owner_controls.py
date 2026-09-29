def sync_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        observe(value)


def sync_else_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        else:
            prepare(value)
        observe(value)


def generator_value_only(values, first, second):
    for value in values:
        if first and second:
            yield value


async def coroutine_await_only(values, first, second):
    for value in values:
        if first and second:
            await observe(value)


def generator_name_call_prefix(values, first, second):
    for value in values:
        prepared = prepare(value)
        if first and second:
            continue
        yield prepared


async def coroutine_name_call_prefix(values, first, second):
    for value in values:
        prepared = prepare(value)
        if first and second:
            continue
        await observe(prepared)


def generator_nested_condition_prefix(values, first, second):
    for value in values:
        if first:
            observe(value)
        if first and second:
            continue
        yield value


async def coroutine_nested_condition_prefix(values, first, second):
    for value in values:
        if first:
            observe(value)
        if first and second:
            continue
        await observe(value)
