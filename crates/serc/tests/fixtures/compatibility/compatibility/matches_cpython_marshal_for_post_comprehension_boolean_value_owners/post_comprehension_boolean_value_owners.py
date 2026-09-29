async def coroutine_dict_or(pairs, left, right):
    copied = {key: value for key, value in pairs if value is not None}
    result = left or right
    return copied, result


async def coroutine_list_and_three(values, left, middle, right):
    copied = [value for value in values]
    result = left and middle and right
    return copied, result


async def coroutine_set_call_gap(values, consume, left, right):
    copied = {value for value in values if value is not None}
    consume(left)
    result = left or right
    return copied, result


async def coroutine_nested_comprehension(values, left, right):
    copied = [inner for value in values for inner in value if inner is not None]
    result = left and right
    return copied, result


def generator_list_or_three(values, left, middle, right):
    copied = [value for value in values if value is not None]
    result = left or middle or right
    yield copied, result


def generator_dict_prior_boolean(pairs, left, middle, right):
    copied = {key: value for key, value in pairs}
    first = left or middle
    result = middle and right
    yield copied, first, result


def generator_set_payload(values, left, middle, right):
    copied = {value for value in values if value is not None}
    result = {"left": left or middle, "right": middle or right}
    yield copied, result


async def async_generator_list_and(values, left, right):
    copied = [value for value in values]
    result = left and right
    yield copied, result


async def async_generator_dict_or_three(pairs, left, middle, right):
    copied = {key: value for key, value in pairs if value is not None}
    result = left or middle or right
    yield copied, result


def ordinary_control(values, left, middle, right):
    copied = [value for value in values if value is not None]
    result = left or middle or right
    return copied, result


async def coroutine_after_await_control(values, ready, left, middle, right):
    await ready()
    copied = [value for value in values]
    result = left and middle and right
    return copied, result


def generator_after_yield_control(values, initial, enabled, consume, left, middle, right):
    yield initial
    copied = [value for value in values if value is not None]
    if enabled:
        consume(left)
    result = left or middle or right
    yield copied, result


async def async_generator_after_yield_control(
    pairs, initial, enabled, consume, left, middle, right
):
    yield initial
    copied = {key: value for key, value in pairs if value is not None}
    if enabled:
        consume(left)
    result = left and middle and right
    yield copied, result


def generator_nested_call_control(values, consume, left, middle, right):
    copied = consume([value for value in values if value is not None])
    result = left or middle or right
    yield copied, result


async def async_generator_nested_tuple_control(pairs, left, middle, right):
    copied = ({key: value for key, value in pairs if value is not None}, left)
    result = left and middle and right
    yield copied, result


async def coroutine_branch_control(values, enabled, left, middle, right):
    if enabled:
        copied = [value for value in values]
    else:
        copied = []
    result = left or middle or right
    return copied, result


def generator_branch_control(values, enabled, left, middle, right):
    if enabled:
        copied = []
    else:
        copied = [value for value in values if value is not None]
    result = left and middle and right
    yield copied, result


async def coroutine_match_control(values, enabled, left, middle, right):
    match enabled:
        case True:
            copied = [value for value in values]
        case _:
            copied = []
    result = left or middle or right
    return copied, result
