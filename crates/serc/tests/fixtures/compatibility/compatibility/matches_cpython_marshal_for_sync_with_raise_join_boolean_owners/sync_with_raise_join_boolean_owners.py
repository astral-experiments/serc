def direct_raise_join(first, second, value, manager):
    with manager:
        if first is None:
            raise RuntimeError
        result = prepare(first or {})
        observe(result)


def consecutive_raise_joins(first, second, value, manager):
    with manager:
        if first is None:
            raise RuntimeError
        if first.ready:
            raise ValueError
        if second is None:
            raise RuntimeError
        result = prepare(first.value or {})
        observe(result)


def assignment_gap_raise_join(first, second, value, manager):
    with manager:
        if first is None:
            raise RuntimeError
        prepared = prepare(second)
        another = value
        result = prepared or {}
        observe(another, result)


def nested_raise_join(first, second, value, manager):
    with manager:
        if first is None:
            if second:
                raise RuntimeError
        result = prepare(second) and value
        observe(result)


def outer_try_raise_join(first, second, value, manager):
    try:
        with manager:
            if first is None:
                raise RuntimeError
            result = prepare(second) or {}
            observe(result)
    except RuntimeError:
        observe(value)


def preceding_return_then_raise_control(first, second, value, manager):
    with manager:
        if first:
            return
        if second:
            raise RuntimeError
        result = first or second
        observe(result)


async def coroutine_raise_control(first, second, value, manager, seed):
    await suspend(seed)
    with manager:
        if first:
            raise RuntimeError
        result = first or second
        observe(result)


def generator_raise_control(first, second, value, manager, seed):
    yield seed
    with manager:
        if first:
            raise RuntimeError
        result = first or second
        observe(result)


async def async_generator_raise_control(first, second, value, manager, seed):
    yield seed
    with manager:
        if first:
            raise RuntimeError
        result = first or second
        observe(result)


def finally_raise_control(first, second, value, manager):
    try:
        with manager:
            if first:
                raise RuntimeError
            result = first or second
            observe(result)
    finally:
        observe(value)


def nested_with_raise_control(first, second, value, outer, inner):
    with outer:
        if first:
            with inner:
                raise RuntimeError
        result = first or second
        observe(result)


def nested_loop_raise_control(first, second, value, values, manager):
    with manager:
        if first:
            for item in values:
                raise RuntimeError
        result = first or second
        observe(result)


def nested_try_raise_control(first, second, value, manager):
    with manager:
        if first:
            try:
                observe(value)
            except RuntimeError:
                raise
        result = first or second
        observe(result)
