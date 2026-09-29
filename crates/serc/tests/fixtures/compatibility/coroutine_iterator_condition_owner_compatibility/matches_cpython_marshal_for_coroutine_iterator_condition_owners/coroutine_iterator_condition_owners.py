async def diagnostics_name(values, get_state):
    result = {}
    for item in values:
        state = get_state(item)
        state_dict = None
        if state:
            state_dict = dict(state.as_dict())
            state_dict.pop("context", None)
        result[item] = state_dict
    return result


async def diagnostics_walrus(values, get_state):
    result = []
    for item in values:
        state_dict = None
        if state := get_state(item):
            state_dict = dict(state.as_dict())
            state_dict.pop("context", None)
        result.append({"item": item, "state": state_dict})
    return result


async def nested_walrus(groups, get_state):
    result = []
    for group in groups:
        nested = []
        for item in group:
            state_dict = None
            if state := get_state(item):
                state_dict = dict(state.as_dict())
            nested.append({"item": item, "state": state_dict})
        result.append(nested)
    return result


async def type_checking_name(values, type_checking, use):
    for item in values:
        device_key = item.device_key
        if type_checking:
            assert device_key is not None
        use(device_key)


async def nested_after_await(groups, wait, get_state, use):
    for group in groups:
        await wait(group)
        for item in group:
            state = get_state(item)
            if state:
                use(state)
            use(item)


async def nested_after_completed_try(groups, get_state, use):
    for group in groups:
        try:
            use(group)
        except Exception:
            pass
        for item in group:
            state = get_state(item)
            if state:
                use(state)
            use(item)


async def loop_else(values, get_state, use):
    for item in values:
        state = get_state(item)
        if state:
            use(state)
        use(item)
    else:
        use(None)


async def awaited_iterable(values, get_values, get_state, use):
    for item in await get_values(values):
        state = get_state(item)
        if state:
            use(state)
        use(item)


async def multiple_conditions(values, get_state, ready, use):
    for item in values:
        state = get_state(item)
        state_dict = None
        if state:
            state_dict = state.as_dict()
        if ready(item):
            use(state_dict)


def sync_control(values, get_state, use):
    for item in values:
        state = get_state(item)
        if state:
            use(state)
        use(item)
