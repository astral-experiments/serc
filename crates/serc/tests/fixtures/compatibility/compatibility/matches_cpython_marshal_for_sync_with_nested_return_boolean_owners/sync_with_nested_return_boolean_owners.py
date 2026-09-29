def nested_with_direct(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                return value
        result = prepare(second) or {}
        observe(result)


def nested_with_else_return(first, second, value, outer, inner):
    with outer:
        if first:
            observe(value)
        else:
            with inner:
                return value
        result = prepare(second) or {}
        observe(result)


def nested_with_assignment_gap(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                return value
        prepared = prepare(second)
        result = prepared or {}
        observe(result)


def nested_with_expression_gap(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                return value
        observe(second)
        result = prepare(second) or {}
        observe(result)


def nested_with_and_value(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                return value
        result = prepare(second) and value
        observe(result)


def nested_with_nested_value(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                return value
        result = first and (prepare(second) or {})
        observe(result)


def nested_with_outer_try(first, second, value, outer, inner):
    try:
        with outer:
            if first:
                with inner:
                    return value
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)


def nested_with_raise_control(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                raise RuntimeError
        result = first or second
        observe(result)


async def nested_async_with_control(first, second, value, outer, inner, seed):
    await suspend(seed)
    with outer:
        if first:
            async with inner:
                return
        result = prepare(first) or prepare(second)
        observe(result)


async def nested_with_coroutine_control(first, second, value, outer, inner, seed):
    await suspend(seed)
    with outer:
        if first:
            with inner:
                return
        observe(value)
        observe(prepare(second) or {})


def nested_with_generator_control(first, second, value, outer, inner, seed):
    yield seed
    with outer:
        if first:
            with inner:
                return
        observe(value)
        observe(prepare(second) or {})


async def nested_with_async_generator_control(first, second, value, outer, inner, seed):
    yield seed
    with outer:
        if first:
            with inner:
                return
        observe(value)
        observe(prepare(second) or {})


def nested_for_return_control(first, second, value, values, outer):
    with outer:
        if first:
            for item in values:
                return item
        result = first or second
        observe(result)


async def nested_try_return_control(first, second, value, outer):
    yield value
    with outer:
        if first:
            try:
                prepare(value)
            except RuntimeError:
                return
        result = first or second
        observe(result)
    yield second
