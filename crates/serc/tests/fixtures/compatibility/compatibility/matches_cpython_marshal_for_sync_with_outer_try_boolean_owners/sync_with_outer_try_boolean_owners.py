def outer_try_direct(first, second, value, manager):
    try:
        with manager:
            if first is None:
                return value
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_assignment_gap(first, second, value, manager):
    try:
        with manager:
            if first is None:
                return value
            prepared = prepare(second)
            result = prepared or {}
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_expression_gap(first, second, value, manager):
    try:
        with manager:
            if first is None:
                return value
            observe(second)
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_nested_return(first, second, value, manager):
    try:
        with manager:
            if first is None:
                if second:
                    return value
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_else_return(first, second, value, manager):
    try:
        with manager:
            if first:
                observe(value)
            else:
                return value
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_and_value(first, second, value, manager):
    try:
        with manager:
            if first is None:
                return value
            result = prepare(second) and value
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_nested_value(first, second, value, manager):
    try:
        with manager:
            if first is None:
                return value
            result = first and (prepare(second) or {})
            observe(result)
    except RuntimeError:
        observe(value)


def outer_try_two_handlers(first, second, value, manager):
    try:
        with manager:
            if first is None:
                return value
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)
    except ValueError:
        observe(first)


def outer_try_loop(first, second, value, values, manager):
    try:
        with manager:
            for value in values:
                if first is None:
                    return value
                result = prepare(second) or {}
                observe(result)
    except RuntimeError:
        observe(value)


def outer_try_raise_control(first, second, value, manager):
    try:
        with manager:
            if first is None:
                raise RuntimeError
            result = first or second
            observe(result)
    except RuntimeError:
        observe(value)


async def outer_try_coroutine_control(first, second, value, manager, seed):
    await suspend(seed)
    try:
        with manager:
            if first:
                return
            observe(value)
            observe(prepare(second) or {})
    except RuntimeError:
        observe(seed)


def outer_try_generator_control(first, second, value, manager, seed):
    yield seed
    try:
        with manager:
            if first:
                return
            observe(value)
            observe(prepare(second) or {})
    except RuntimeError:
        observe(seed)


async def outer_try_async_generator_control(first, second, value, manager, seed):
    yield seed
    try:
        with manager:
            if first:
                return
            observe(value)
            observe(prepare(second) or {})
    except RuntimeError:
        observe(seed)
