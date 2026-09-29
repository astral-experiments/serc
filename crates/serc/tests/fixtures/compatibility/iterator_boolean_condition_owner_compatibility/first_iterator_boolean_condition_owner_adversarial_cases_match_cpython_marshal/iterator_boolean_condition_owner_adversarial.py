def generator_outer_except(values, first, second):
    try:
        for value in values:
            if first and second:
                continue
            yield value
    except RuntimeError:
        recover()


async def coroutine_outer_finally(values, first, second):
    try:
        for value in values:
            if first and second:
                continue
            await observe(value)
    finally:
        finish()


def generator_nested_four_assign_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            value = prepare(value)
        else:
            value = fallback(value)
        yield value


async def coroutine_nested_four_assign_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            value = prepare(value)
        else:
            value = fallback(value)
        await observe(value)


def generator_nested_four_raise(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            raise RuntimeError(value)
        yield value


async def coroutine_nested_four_return(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            return value
        await observe(value)


def generator_nested_four_break(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            break
        yield value


async def coroutine_nested_four_elif(
    values,
    first,
    second,
    third,
    fourth,
    alternative,
):
    for value in values:
        if first and (second or third) and fourth:
            continue
        elif alternative:
            value = prepare(value)
        await observe(value)


def generator_nested_four_for_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            continue
        yield value
    else:
        yield None


async def coroutine_nested_four_for_else(values, first, second, third, fourth):
    for value in values:
        if first and (second or third) and fourth:
            continue
        await observe(value)
    else:
        await observe(None)


def generator_mixed_or_and(values, first, second, third, fourth):
    for value in values:
        if first or (second and third) or fourth:
            continue
        yield value


async def coroutine_mixed_or_and(values, first, second, third, fourth):
    for value in values:
        if first or (second and third) or fourth:
            continue
        await observe(value)


def generator_inner_try(values, first, second):
    for value in values:
        try:
            if first and second:
                continue
            yield value
        except RuntimeError:
            recover()


async def coroutine_inner_try(values, first, second):
    for value in values:
        try:
            if first and second:
                continue
            await observe(value)
        except RuntimeError:
            recover()


def generator_expression_control(values, first, second):
    return (value for value in values if first and second)


def generator_sync_with_control(manager, values, first, second):
    with manager:
        for value in values:
            if first and second:
                continue
            yield value


async def coroutine_sync_with_control(manager, values, first, second):
    with manager:
        for value in values:
            if first and second:
                continue
            await observe(value)
