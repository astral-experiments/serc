async def protected_loop(values, get_state, use):
    try:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)
    except Exception:
        use(None)

async def with_loop(values, get_state, use, manager):
    with manager:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def async_with_loop(values, get_state, use, manager):
    async with manager:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def handler_loop(values, get_state, use):
    try:
        use(None)
    except Exception:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def finally_loop(values, get_state, use):
    try:
        use(None)
    finally:
        for item in values:
            state = get_state(item)
            if state:
                use(state)
            use(item)

async def async_for_control(values, get_state, use):
    async for item in values:
        state = get_state(item)
        if state:
            use(state)
        use(item)

def generator_control(values, get_state, use):
    for item in values:
        state = get_state(item)
        if state:
            use(state)
        yield item

async def async_generator_control(values, get_state, use):
    for item in values:
        state = get_state(item)
        if state:
            use(state)
        yield item
