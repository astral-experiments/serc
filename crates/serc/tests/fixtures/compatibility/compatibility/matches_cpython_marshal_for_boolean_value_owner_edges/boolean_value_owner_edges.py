def generator_expression_stale(items):
    return (f(g(h(item or 0))) for item in items)


async def async_generator_expression_stale(items):
    return (f(g(h(item or 0))) async for item in items)


def plain_try_stale(value):
    try:
        result = f(g(h(value or 0)))
    except Error:
        recover()
    return result


def same_line_try(value):
    try: result = value or 0
    except Error: recover()
    return result


async def coroutine_stale(value):
    await source
    result = f(g(h(value or 0)))
    consume(result)


async def coroutine_same_line_try(value):
    await source
    try: result = value or 0
    except Error: recover()
    return result


async def named_handler(value):
    try:
        await source
    except Error as error:
        result = value or 0
        consume(error, result)


async def unnamed_handler(value):
    try:
        await source
    except Error:
        result = f(value or 0)
        consume(result)


async def unnamed_handler_stale(value):
    try:
        await source
    except Error:
        result = f(g(h(value or 0)))
        consume(result)


async def after_async_for(items, value):
    async for item in items:
        consume(item)
    try:
        result = value or 0
        consume(result)
    finally:
        cleanup()


async def after_async_comprehension(items, value):
    [item async for item in items]
    try:
        result = value or 0
        consume(result)
    finally:
        cleanup()


async def async_generator_after_await(value):
    await source
    try:
        result = value or 0
        consume(result)
    finally:
        cleanup()
    yield result


async def coroutine_finally_two_stale_slots(value):
    try:
        await source
    finally:
        result = f(g(value or 0))


def generator_target_folded_membership(items, value):
    return (item in (1, 2) and value for item in items)


def generator_freevar_folded_membership(items, needle, value):
    return (needle in (1, 2) and value for item in items)


def generator_freevar_three_folded_membership(items, needle, value):
    return (needle in (1, 2, 3) and value for item in items)


def generator_freevar_equality(items, needle, value):
    return (needle == 1 and value for item in items)


def generator_target_folded_arithmetic(items, value):
    return (item == (1 + 2 + 3) and value for item in items)


def generator_freevar_folded_arithmetic(items, needle, value):
    return (needle == (1 + 2) and value for item in items)


def generator_folded_chained_compare(items, value):
    return (item < (1 + 2 + 3) < 10 and value for item in items)


def generator_freevar_large_folded_tuple(items, needle, value):
    return (
        needle
        == (
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            11,
            12,
            13,
            14,
            15,
            16,
            17,
            18,
            19,
            20,
            21,
            22,
            23,
            24,
            25,
            26,
            27,
            28,
            29,
            30,
        )
        and value
        for item in items
    )


def generator_nested_fold_boundary(items, value):
    return (sink(-(1 + 2), (item or value)) and value for item in items)


def plain_try_nested_fold_boundary(item, value):
    try:
        result = sink(-(1 + 2), (item or value)) and value
    except Error:
        recover()
    return result


def generator_folded_collections(items, value):
    left = ([1, 2, 3, 4, 5] and value for item in items)
    right = ({1 + 2, 4, 5} and value for item in items)
    return left, right


def generator_folded_membership_not(items, needle, value):
    return (needle in [not 1] and value for item in items)


def generator_folded_tuple_not(items, value):
    return ((not 1,) and value for item in items)


def generator_folded_tuple_not_multiline(items, value):
    return (
        (
            not 1,
        )
        and value
        for item in items
    )


def generator_call_pack(items, kwargs, value):
    return (sink(1 + 2, 3, **kwargs) and value for item in items)
