def direct_return_generator_calls(first, second, manager, values):
    with manager:
        if not first:
            return False
        prepared = prepare(second)
        result = any(item == first for item in values) or any(
            item == second for item in values
        )
        observe(prepared, result)
    return True


def generator_calls_following_return(first, second, manager, values):
    with manager:
        state = prepare(second)
        if not first:
            return False
        prepared = prepare(state)
        result = any(item == first for item in values) or any(
            item == second for item in values
        )
        if result:
            return False
        observe(prepared)
    return True


def generator_calls_multiple_returns(first, second, manager, values):
    with manager:
        state = prepare(second)
        if not first:
            return False
        prepared = prepare(state)
        result = any(item == first for item in values) or any(
            item == second for item in values
        )
        if prepared or result:
            return False
        if first == second:
            return False
        observe(state)
    return True


def filtered_nested_generator_calls(first, second, manager, groups):
    with manager:
        if first:
            if second:
                return False
        prepared = prepare(second)
        result = any(
            item == first for group in groups for item in group if keep(item)
        ) or any(item == second for group in groups for item in group if keep(item))
        observe(prepared, result)
    return True


def method_generator_calls(first, second, manager, source):
    with manager:
        state = source.state(first)
        if not state.ready:
            return False
        prepared = source.prepare(state)
        result = any(item.key == first for item in source.active()) or any(
            item.key == first for item in source.pending()
        )
        if prepared.ready or prepared.busy or result:
            return False
        observe(prepared)
    return True


async def coroutine_generator_calls(first, second, manager, values, seed):
    await suspend(seed)
    with manager:
        if not first:
            return False
        prepared = prepare(second)
        result = all(item != first for item in values) or any(
            item == second for item in values
        )
        observe(prepared, result)
    return True


def generator_generator_calls(first, second, manager, values, seed):
    yield seed
    with manager:
        if not first:
            return False
        prepared = prepare(second)
        result = any(item == first for item in values) and any(
            item == second for item in values
        )
        observe(prepared, result)
    return True


async def async_generator_generator_calls(first, second, manager, values, seed):
    yield seed
    with manager:
        if not first:
            return
        prepared = prepare(second)
        result = any(not item for item in values) or any(
            item == second for item in values
        )
        observe(prepared, result)


def no_return_generator_control(first, second, manager, values):
    with manager:
        prepared = prepare(second)
        result = any(item == first for item in values) or any(
            item == second for item in values
        )
        observe(prepared, result)
    return True


def raise_generator_control(first, second, manager, values):
    with manager:
        if not first:
            raise RuntimeError
        prepared = prepare(second)
        result = any(item == first for item in values) or any(
            item == second for item in values
        )
        observe(prepared, result)
    return True


def plain_calls_control(first, second, manager):
    with manager:
        if not first:
            return False
        prepared = prepare(second)
        result = check(first) or check(second)
        observe(prepared, result)
    return True
