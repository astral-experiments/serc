async def coroutine_one(items):
    for item in items:
        state = item.get()
        assert state
        assert state.enabled


async def coroutine_two(items):
    for item in items:
        state = item.get()
        enabled = state.enabled
        assert enabled
        consume(state)


async def coroutine_three(items):
    for item in items:
        state = item.get()
        enabled = state.enabled
        ready = state.ready
        assert ready
        assert enabled


async def rhs_shapes(items):
    for item in items:
        state = item.state
        value = state[0]
        key = f'{value}'
        assert key
        assert state.enabled


async def named_assertion(items):
    for item in items:
        state = item.get()
        assert (result := state.result)
        assert result.enabled


async def attribute_assertion(items):
    for item in items:
        state = item.get()
        assert state.enabled
        assert state.ready


def generator(items):
    for item in items:
        state = item.get()
        assert state
        yield state


async def asynchronous_iterator(items):
    async for item in items:
        state = item.get()
        assert state
        consume(state)


async def awaited_assignment_control(items):
    for item in items:
        state = await item.get()
        assert state
        assert state.enabled


async def async_comprehension_control(items):
    for item in items:
        state = [value async for value in item.values()]
        assert state
        assert item.enabled


async def binary_assignment_control(items):
    for item in items:
        state = item + item
        assert state
        assert state.enabled


async def boolean_call_argument_control(items, fallback):
    for item in items:
        state = item.get(fallback or item)
        assert state
        assert state.enabled


async def comprehension_call_argument_control(items):
    for item in items:
        state = item.get([value for value in item.values if value])
        assert state
        assert state.enabled


async def multiple_target_control(items):
    for item in items:
        state = other = item.get()
        assert state
        assert other.enabled


async def unpack_control(items):
    for item in items:
        state, other = item.get()
        assert state
        assert other.enabled


async def augmented_assignment_control(items, state):
    for item in items:
        state += item.get()
        assert state
        assert item.enabled


async def expression_control(items):
    for item in items:
        consume(item)
        assert item
        assert item.enabled


async def conditional_control(items, enabled):
    for item in items:
        if enabled:
            state = item.get()
        else:
            state = item.other()
        assert state
        assert state.enabled


async def terminal_control(items):
    for item in items:
        state = item.get()
        assert state


def ordinary_control(items):
    for item in items:
        state = item.get()
        assert state
        assert state.enabled
