def gen_and_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        yield value


def gen_or_continue(values, first, second):
    for value in values:
        if first or second:
            continue
        yield value


def gen_not_call_continue(values, first, checker):
    for value in values:
        if first and not checker(value):
            continue
        yield value


def gen_compare_continue(values, first, allowed):
    for value in values:
        if first and value not in allowed:
            continue
        yield value


def gen_three_continue(values, first, second, third):
    for value in values:
        if first and second and third:
            continue
        yield value


def gen_nested_continue(values, first, second, third):
    for value in values:
        if first and (second or third):
            continue
        yield value


def gen_body_continue(values, first, second):
    for value in values:
        if first and second:
            observe(value)
            continue
        yield value


def gen_else_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        else:
            observe(value)
        yield value


def gen_assign(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        yield value


def gen_assign_else(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        else:
            value = fallback(value)
        yield value


async def coro_and_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        await observe(value)


async def coro_or_continue(values, first, second):
    for value in values:
        if first or second:
            continue
        await observe(value)


async def coro_not_call_continue(values, first, checker):
    for value in values:
        if first and not checker(value):
            continue
        await observe(value)


async def coro_compare_continue(values, first, allowed):
    for value in values:
        if first and value not in allowed:
            continue
        await observe(value)


async def coro_three_continue(values, first, second, third):
    for value in values:
        if first and second and third:
            continue
        await observe(value)


async def coro_nested_continue(values, first, second, third):
    for value in values:
        if first and (second or third):
            continue
        await observe(value)


async def coro_body_continue(values, first, second):
    for value in values:
        if first and second:
            prepare(value)
            continue
        await observe(value)


async def coro_else_continue(values, first, second):
    for value in values:
        if first and second:
            continue
        else:
            prepare(value)
        await observe(value)


async def coro_assign(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        await observe(value)


async def coro_assign_else(values, first, second):
    for value in values:
        if first and second:
            value = prepare(value)
        else:
            value = fallback(value)
        await observe(value)
